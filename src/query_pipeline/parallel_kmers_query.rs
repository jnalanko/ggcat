use crate::colors::colors_manager::color_types::{
    MinimizerBucketingSeqColorDataType, SingleKmerColorDataType,
};
use crate::colors::colors_manager::{ColorsManager, MinimizerBucketingSeqColorData};
use crate::config::{
    BucketIndexType, SwapPriority, DEFAULT_PER_CPU_BUFFER_SIZE, MINIMUM_SUBBUCKET_KMERS_COUNT,
    RESPLITTING_MAX_K_M_DIFFERENCE,
};
use crate::hashes::HashFunctionFactory;
use crate::hashes::{ExtendableHashTraitType, MinimizerHashFunctionFactory};
use crate::hashes::{HashFunction, HashableSequence};
use crate::io::concurrent::temp_reads::extra_data::{
    SequenceExtraData, SequenceExtraDataTempBufferManagement,
};
use crate::io::varint::{decode_varint, encode_varint};
use crate::pipeline_common::kmers_transform::processor::KmersTransformProcessor;
use crate::pipeline_common::kmers_transform::{
    KmersTransform, KmersTransformExecutorFactory, KmersTransformFinalExecutor,
    KmersTransformMapProcessor, KmersTransformPreprocessor,
};
use crate::pipeline_common::minimizer_bucketing::{
    MinimizerBucketingCommonData, MinimizerBucketingExecutorFactory,
};
use crate::query_pipeline::counters_sorting::CounterEntry;
use crate::query_pipeline::querier_minimizer_bucketing::QuerierMinimizerBucketingExecutorFactory;
use crate::query_pipeline::QueryPipeline;
use crate::utils::compressed_read::CompressedReadIndipendent;
use crate::utils::get_memory_mode;
use crate::CompressedRead;
use byteorder::{ReadBytesExt, WriteBytesExt};
use hashbrown::HashMap;
use parallel_processor::buckets::concurrent::{BucketsThreadBuffer, BucketsThreadDispatcher};
use parallel_processor::buckets::writers::lock_free_binary_writer::LockFreeBinaryWriter;
use parallel_processor::buckets::MultiThreadBuckets;
use parallel_processor::execution_manager::memory_tracker::MemoryTracker;
use parallel_processor::execution_manager::objects_pool::PoolObjectTrait;
use parallel_processor::execution_manager::packet::{Packet, PacketTrait};
use parallel_processor::phase_times_monitor::PHASES_TIMES_MONITOR;
use std::cmp::min;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum QueryKmersReferenceData<CX: MinimizerBucketingSeqColorData> {
    Graph(CX),
    Query(NonZeroU64),
}

impl<CX: MinimizerBucketingSeqColorData> SequenceExtraDataTempBufferManagement<(CX::TempBuffer,)>
    for QueryKmersReferenceData<CX>
{
    #[inline(always)]
    fn new_temp_buffer() -> (CX::TempBuffer,) {
        (CX::new_temp_buffer(),)
    }

    #[inline(always)]
    fn clear_temp_buffer(buffer: &mut (CX::TempBuffer,)) {
        CX::clear_temp_buffer(&mut buffer.0);
    }

    #[inline(always)]
    fn copy_extra_from(extra: Self, src: &(CX::TempBuffer,), dst: &mut (CX::TempBuffer,)) -> Self {
        match extra {
            QueryKmersReferenceData::Graph(color) => {
                QueryKmersReferenceData::Graph(CX::copy_extra_from(color, &src.0, &mut dst.0))
            }
            QueryKmersReferenceData::Query(index) => QueryKmersReferenceData::Query(index),
        }
    }
}

impl<CX: MinimizerBucketingSeqColorData> SequenceExtraData for QueryKmersReferenceData<CX> {
    type TempBuffer = (CX::TempBuffer,);

    #[inline(always)]
    fn decode_extended(buffer: &mut Self::TempBuffer, reader: &mut impl Read) -> Option<Self> {
        match reader.read_u8().ok()? {
            0 => Some(Self::Graph(CX::decode_extended(&mut buffer.0, reader)?)),
            _ => Some(Self::Query(
                NonZeroU64::new(decode_varint(|| reader.read_u8().ok())? + 1).unwrap(),
            )),
        }
    }

    #[inline(always)]
    fn encode_extended(&self, buffer: &Self::TempBuffer, writer: &mut impl Write) {
        match self {
            Self::Graph(cx) => {
                writer.write_u8(0).unwrap();
                CX::encode_extended(cx, &buffer.0, writer);
            }
            Self::Query(val) => {
                writer.write_u8(1).unwrap();
                encode_varint(|bytes| writer.write_all(bytes), val.get() - 1).unwrap();
            }
        }
    }

    #[inline(always)]
    fn max_size(&self) -> usize {
        match self {
            Self::Graph(cx) => cx.max_size() + 1,
            Self::Query(_) => 10 + 1,
        }
    }
}

struct GlobalQueryMergeData {
    k: usize,
    m: usize,
    counters_buckets: Arc<MultiThreadBuckets<LockFreeBinaryWriter>>,
    global_resplit_data: Arc<MinimizerBucketingCommonData<()>>,
}

struct ParallelKmersQueryFactory<
    H: MinimizerHashFunctionFactory,
    MH: HashFunctionFactory,
    CX: ColorsManager,
>(PhantomData<(H, MH, CX)>);

impl<H: MinimizerHashFunctionFactory, MH: HashFunctionFactory, CX: ColorsManager>
    KmersTransformExecutorFactory for ParallelKmersQueryFactory<H, MH, CX>
{
    type SequencesResplitterFactory = QuerierMinimizerBucketingExecutorFactory<H, CX>;
    type GlobalExtraData = GlobalQueryMergeData;
    type AssociatedExtraData = QueryKmersReferenceData<MinimizerBucketingSeqColorDataType<CX>>;

    type PreprocessorType = ParallelKmersQueryPreprocessor<H, MH, CX>;
    type MapProcessorType = ParallelKmersQueryMapProcessor<H, MH, CX>;
    type FinalExecutorType = ParallelKmersQueryFinalExecutor<H, MH, CX>;

    #[allow(non_camel_case_types)]
    type FLAGS_COUNT = typenum::U0;

    fn new_resplitter(
        global_data: &Arc<Self::GlobalExtraData>,
    ) -> <Self::SequencesResplitterFactory as MinimizerBucketingExecutorFactory>::ExecutorType {
        QuerierMinimizerBucketingExecutorFactory::new(&global_data.global_resplit_data)
    }

    fn new_preprocessor(_global_data: &Arc<Self::GlobalExtraData>) -> Self::PreprocessorType {
        Self::PreprocessorType {
            _phantom: PhantomData,
        }
    }

    fn new_map_processor(
        _global_data: &Arc<Self::GlobalExtraData>,
        _mem_tracker: MemoryTracker<KmersTransformProcessor<Self>>,
    ) -> Self::MapProcessorType {
        Self::MapProcessorType {
            map_packet: None,
            _phantom: PhantomData,
        }
    }

    fn new_final_executor(global_data: &Arc<Self::GlobalExtraData>) -> Self::FinalExecutorType {
        let counters_buffers = BucketsThreadBuffer::new(
            DEFAULT_PER_CPU_BUFFER_SIZE,
            global_data.counters_buckets.count(),
        );

        Self::FinalExecutorType {
            counters_tmp: BucketsThreadDispatcher::new(
                &global_data.counters_buckets,
                counters_buffers,
            ),
            query_map: HashMap::new(),
            _phantom: PhantomData,
        }
    }
}

struct ParallelKmersQueryPreprocessor<
    H: HashFunctionFactory,
    MH: HashFunctionFactory,
    CX: ColorsManager,
> {
    _phantom: PhantomData<(H, MH, CX)>,
}

impl<H: MinimizerHashFunctionFactory, MH: HashFunctionFactory, CX: ColorsManager>
    KmersTransformPreprocessor<ParallelKmersQueryFactory<H, MH, CX>>
    for ParallelKmersQueryPreprocessor<H, MH, CX>
{
    fn get_sequence_bucket<C>(
        &self,
        global_data: &<ParallelKmersQueryFactory<H, MH, CX> as KmersTransformExecutorFactory>::GlobalExtraData,
        seq_data: &(u8, u8, C, CompressedRead),
    ) -> BucketIndexType {
        let read = &seq_data.3;

        let hashes = H::new(read.sub_slice(0..global_data.k), global_data.m);

        let minimizer = hashes
            .iter()
            .min_by_key(|k| H::get_full_minimizer(k.to_unextendable()))
            .unwrap();

        H::get_second_bucket(minimizer.to_unextendable())
    }
}

struct ParallelKmersQueryMapPacket<MH: HashFunctionFactory, CX: Sync + Send + 'static> {
    phmap: HashMap<MH::HashTypeUnextendable, CX>,
    query_reads: Vec<(u64, MH::HashTypeUnextendable)>,
}

impl<MH: HashFunctionFactory, CX: Sync + Send + 'static> PoolObjectTrait
    for ParallelKmersQueryMapPacket<MH, CX>
{
    type InitData = ();

    fn allocate_new(_init_data: &Self::InitData) -> Self {
        Self {
            phmap: HashMap::new(),
            query_reads: Vec::new(),
        }
    }

    fn reset(&mut self) {
        self.phmap.clear();
        self.query_reads.clear();
    }
}
impl<MH: HashFunctionFactory, CX: Sync + Send + 'static> PacketTrait
    for ParallelKmersQueryMapPacket<MH, CX>
{
    fn get_size(&self) -> usize {
        (self.phmap.len() + self.query_reads.len()) * 16 // TODO: Compute correct values
    }
}

struct ParallelKmersQueryMapProcessor<
    H: MinimizerHashFunctionFactory,
    MH: HashFunctionFactory,
    CX: ColorsManager,
> {
    map_packet: Option<Packet<ParallelKmersQueryMapPacket<MH, SingleKmerColorDataType<CX>>>>,
    _phantom: PhantomData<(H, CX)>,
}

impl<H: MinimizerHashFunctionFactory, MH: HashFunctionFactory, CX: ColorsManager>
    KmersTransformMapProcessor<ParallelKmersQueryFactory<H, MH, CX>>
    for ParallelKmersQueryMapProcessor<H, MH, CX>
{
    type MapStruct = ParallelKmersQueryMapPacket<MH, SingleKmerColorDataType<CX>>;

    fn process_group_start(
        &mut self,
        map_struct: Packet<Self::MapStruct>,
        _global_data: &GlobalQueryMergeData,
    ) {
        self.map_packet = Some(map_struct);
    }

    fn process_group_batch_sequences(
        &mut self,
        global_data: &GlobalQueryMergeData,
        batch: &Vec<(
            u8,
            QueryKmersReferenceData<MinimizerBucketingSeqColorDataType<CX>>,
            CompressedReadIndipendent,
        )>,
        extra_data_buffer: &<QueryKmersReferenceData<MinimizerBucketingSeqColorDataType<CX>> as SequenceExtraData>::TempBuffer,
        ref_sequences: &Vec<u8>,
    ) {
        let k = global_data.k;
        let map_packet = self.map_packet.as_mut().unwrap();

        for (_, sequence_type, read) in batch.iter() {
            let hashes = MH::new(read.as_reference(ref_sequences), k);

            match sequence_type {
                QueryKmersReferenceData::Graph(col_info) => {
                    for (hash, color) in hashes
                        .iter()
                        .zip(col_info.get_iterator(&extra_data_buffer.0))
                    {
                        map_packet.phmap.insert(hash.to_unextendable(), color);
                    }
                }
                QueryKmersReferenceData::Query(index) => {
                    for hash in hashes.iter() {
                        map_packet
                            .query_reads
                            .push((index.get(), hash.to_unextendable()));
                    }
                }
            }
        }
    }

    fn process_group_finalize(
        &mut self,
        _global_data: &GlobalQueryMergeData,
    ) -> Packet<Self::MapStruct> {
        self.map_packet.take().unwrap()
    }
}

struct ParallelKmersQueryFinalExecutor<
    H: MinimizerHashFunctionFactory,
    MH: HashFunctionFactory,
    CX: ColorsManager,
> {
    counters_tmp: BucketsThreadDispatcher<LockFreeBinaryWriter>,
    query_map: HashMap<(u64, SingleKmerColorDataType<CX>), u64>,
    _phantom: PhantomData<(H, MH, CX)>,
}

impl<H: MinimizerHashFunctionFactory, MH: HashFunctionFactory, CX: ColorsManager>
    KmersTransformFinalExecutor<ParallelKmersQueryFactory<H, MH, CX>>
    for ParallelKmersQueryFinalExecutor<H, MH, CX>
{
    type MapStruct = ParallelKmersQueryMapPacket<MH, SingleKmerColorDataType<CX>>;

    fn process_map(
        &mut self,
        _global_data: &GlobalQueryMergeData,
        map_struct: Packet<Self::MapStruct>,
    ) {
        let map_struct = map_struct.deref();

        for (query_index, kmer_hash) in &map_struct.query_reads {
            if let Some(entry_color) = map_struct.phmap.get(&kmer_hash) {
                *self
                    .query_map
                    .entry((*query_index, entry_color.clone()))
                    .or_insert(0) += 1;
            }
        }

        for ((query_index, color_index), counter) in self.query_map.drain() {
            self.counters_tmp.add_element(
                (query_index % 0xFF) as BucketIndexType,
                &color_index,
                &CounterEntry {
                    query_index,
                    counter,
                    _phantom: PhantomData,
                },
            )
        }
    }

    fn finalize(self, _global_data: &GlobalQueryMergeData) {
        self.counters_tmp.finalize();
    }
}

impl QueryPipeline {
    pub fn parallel_kmers_counting<
        H: MinimizerHashFunctionFactory,
        MH: HashFunctionFactory,
        CX: ColorsManager,
        P: AsRef<Path> + Sync,
    >(
        file_inputs: Vec<PathBuf>,
        buckets_counters_path: PathBuf,
        buckets_count: usize,
        out_directory: P,
        k: usize,
        m: usize,
        threads_count: usize,
    ) -> Vec<PathBuf> {
        PHASES_TIMES_MONITOR
            .write()
            .start_phase("phase: kmers counting".to_string());

        let counters_buckets = Arc::new(MultiThreadBuckets::<LockFreeBinaryWriter>::new(
            buckets_count,
            out_directory.as_ref().join("counters"),
            &(
                get_memory_mode(SwapPriority::QueryCounters),
                LockFreeBinaryWriter::CHECKPOINT_SIZE_UNLIMITED,
            ),
        ));

        let global_data = Arc::new(GlobalQueryMergeData {
            k,
            m,
            counters_buckets,
            global_resplit_data: Arc::new(MinimizerBucketingCommonData::new(
                k,
                if k > RESPLITTING_MAX_K_M_DIFFERENCE + 1 {
                    k - RESPLITTING_MAX_K_M_DIFFERENCE
                } else {
                    min(m, 2)
                }, // m
                buckets_count,
                1,
                (),
            )),
        });

        KmersTransform::<ParallelKmersQueryFactory<H, MH, CX>>::new(
            file_inputs,
            out_directory.as_ref(),
            buckets_counters_path,
            buckets_count,
            global_data.clone(),
            threads_count,
            k,
            m,
            (MINIMUM_SUBBUCKET_KMERS_COUNT / k) as u64,
        )
        .parallel_kmers_transform();

        let global_data =
            Arc::try_unwrap(global_data).unwrap_or_else(|_| panic!("Cannot unwrap global data!"));
        global_data.counters_buckets.finalize()
    }
}
