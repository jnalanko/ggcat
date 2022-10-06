use crate::colors_manager::ColorsMergeManager;
use crate::colors_memmap_writer::ColorsMemMapWriter;
use crate::DefaultColorsSerializer;
use byteorder::ReadBytesExt;
use config::ColorIndexType;
use hashbrown::HashMap;
use hashes::ExtendableHashTraitType;
use hashes::{HashFunction, HashFunctionFactory, HashableSequence};
use io::compressed_read::CompressedRead;
use io::concurrent::temp_reads::extra_data::{
    SequenceExtraData, SequenceExtraDataTempBufferManagement,
};
use io::varint::{decode_varint, encode_varint, VARINT_MAX_SIZE};
use std::collections::VecDeque;
use std::io::{Cursor, Read, Write};
use std::marker::PhantomData;
use std::mem::size_of;
use std::ops::Range;
use std::path::Path;
use structs::map_entry::{MapEntry, COUNTER_BITS};

pub struct MultipleColorsManager<H: HashFunctionFactory> {
    last_color: ColorIndexType,
    sequences: Vec<u8>,
    kmers_count: usize,
    sequences_count: usize,
    temp_colors_buffer: Vec<ColorIndexType>,
    _phantom: PhantomData<H>,
}

const VISITED_BIT: usize = 1 << (COUNTER_BITS - 1);

impl<H: HashFunctionFactory> ColorsMergeManager<H> for MultipleColorsManager<H> {
    type SingleKmerColorDataType = ColorIndexType;
    type GlobalColorsTableWriter = ColorsMemMapWriter<DefaultColorsSerializer>;
    type GlobalColorsTableReader = ();

    fn create_colors_table(
        path: impl AsRef<Path>,
        color_names: Vec<String>,
    ) -> Self::GlobalColorsTableWriter {
        ColorsMemMapWriter::new(path, color_names)
    }

    fn open_colors_table(_path: impl AsRef<Path>) -> Self::GlobalColorsTableReader {
        ()
    }

    fn print_color_stats(global_colors_table: &Self::GlobalColorsTableWriter) {
        global_colors_table.print_stats();
    }

    type ColorsBufferTempStructure = Self;

    fn allocate_temp_buffer_structure() -> Self::ColorsBufferTempStructure {
        Self {
            last_color: 0,
            sequences: vec![],
            kmers_count: 0,
            sequences_count: 0,
            temp_colors_buffer: vec![],
            _phantom: PhantomData,
        }
    }

    fn reinit_temp_buffer_structure(data: &mut Self::ColorsBufferTempStructure) {
        data.sequences.clear();
        data.temp_colors_buffer.clear();
        data.kmers_count = 0;
        data.sequences_count = 0;
    }

    fn add_temp_buffer_structure_el(
        data: &mut Self::ColorsBufferTempStructure,
        kmer_color: &ColorIndexType,
        _el: (usize, H::HashTypeUnextendable),
        _entry: &mut MapEntry<Self::HashMapTempColorIndex>,
    ) {
        data.last_color = *kmer_color;
    }

    #[inline(always)]
    fn add_temp_buffer_sequence(
        data: &mut Self::ColorsBufferTempStructure,
        sequence: CompressedRead,
    ) {
        data.sequences
            .extend_from_slice(&data.last_color.to_ne_bytes());
        encode_varint(
            |b| data.sequences.extend_from_slice(b),
            sequence.bases_count() as u64,
        );
        sequence.copy_to_buffer(&mut data.sequences);
    }

    type HashMapTempColorIndex = ();

    fn new_color_index() -> Self::HashMapTempColorIndex {
        ()
    }

    fn process_colors(
        global_colors_table: &Self::GlobalColorsTableWriter,
        data: &mut Self::ColorsBufferTempStructure,
        map: &mut HashMap<H::HashTypeUnextendable, MapEntry<Self::HashMapTempColorIndex>>,
        k: usize,
        min_multiplicity: usize,
    ) {
        let mut stream = Cursor::new(&data.sequences);

        let mut color_buf = [0; size_of::<ColorIndexType>()];
        let mut read_buf = vec![];

        let mut last_partition = 0..0;
        let mut last_color = 0;

        loop {
            if stream.read(&mut color_buf).unwrap() == 0 {
                break;
            }

            let color = ColorIndexType::from_ne_bytes(color_buf);
            let read_length = decode_varint(|| stream.read_u8().ok()).unwrap() as usize;
            let read_bytes_count = (read_length + 3) / 4;

            read_buf.clear();
            read_buf.reserve(read_bytes_count);
            unsafe { read_buf.set_len(read_bytes_count) };
            stream.read_exact(&mut read_buf[..]).unwrap();

            let read = CompressedRead::new_from_compressed(&read_buf[..], read_length);

            let hashes = H::new(read, k);

            data.temp_colors_buffer
                .reserve(data.kmers_count + data.sequences_count);

            for kmer_hash in hashes.iter() {
                let entry = map.get_mut(&kmer_hash.to_unextendable()).unwrap();

                if entry.get_kmer_multiplicity() < min_multiplicity {
                    continue;
                }

                let mut entry_count = entry.get_counter();

                if entry_count & VISITED_BIT == 0 {
                    let colors_count = entry_count;
                    let start_temp_color_index = data.temp_colors_buffer.len();

                    entry_count = VISITED_BIT | start_temp_color_index;
                    entry.set_counter_after_check(entry_count);

                    data.temp_colors_buffer
                        .resize(data.temp_colors_buffer.len() + colors_count + 1, 0);

                    println!(
                        "Add color subset {} at {}",
                        colors_count,
                        data.temp_colors_buffer.len()
                    );

                    data.temp_colors_buffer[start_temp_color_index] = 1;
                }

                let position = entry_count & !VISITED_BIT;

                let col_count = data.temp_colors_buffer[position] as usize;
                data.temp_colors_buffer[position] += 1;

                println!(
                    "Update color subset at {} + {} < {}",
                    position,
                    col_count,
                    data.temp_colors_buffer.len()
                );

                assert_eq!(data.temp_colors_buffer[position + col_count], 0);
                data.temp_colors_buffer[position + col_count] = color;

                let has_all_colors = (position + col_count + 1) == data.temp_colors_buffer.len()
                    || data.temp_colors_buffer[position + col_count + 1] != 0;

                // All colors were added, let's assign the final color
                if has_all_colors {
                    let colors_range =
                        &mut data.temp_colors_buffer[(position + 1)..(position + col_count + 2)];

                    colors_range.sort_unstable();

                    // Get the new partition indexes, start to dedup last element
                    let new_partition =
                        (position + 1)..(position + 1 + colors_range.partition_dedup().0.len());

                    let unique_colors = &data.temp_colors_buffer[new_partition.clone()];

                    // Assign the subset color index to the current kmer
                    if unique_colors != &data.temp_colors_buffer[last_partition.clone()] {
                        last_color = global_colors_table.get_id(unique_colors);
                        last_partition = new_partition;
                    }

                    entry.set_counter_after_check(VISITED_BIT | (last_color as usize));
                }
            }
        }
    }

    type PartialUnitigsColorStructure = UnitigColorDataSerializer;
    type TempUnitigColorStructure = DefaultUnitigsTempColorData;

    fn alloc_unitig_color_structure() -> Self::TempUnitigColorStructure {
        DefaultUnitigsTempColorData {
            colors: VecDeque::new(),
        }
    }

    fn reset_unitig_color_structure(ts: &mut Self::TempUnitigColorStructure) {
        ts.colors.clear();
    }

    fn extend_forward(
        ts: &mut Self::TempUnitigColorStructure,
        entry: &MapEntry<Self::HashMapTempColorIndex>,
    ) {
        let kmer_color = (entry.get_counter() & !VISITED_BIT) as ColorIndexType;

        if let Some(back_ts) = ts.colors.back_mut() && back_ts.0 == kmer_color {
                    back_ts.1 += 1;
            } else {
                ts.colors.push_back((kmer_color, 1));
            }
    }

    fn extend_backward(
        ts: &mut Self::TempUnitigColorStructure,
        entry: &MapEntry<Self::HashMapTempColorIndex>,
    ) {
        let kmer_color = (entry.get_counter() & !VISITED_BIT) as ColorIndexType;

        if let Some(front_ts) = ts.colors.front_mut()
                && front_ts.0 == kmer_color {
                front_ts.1 += 1;
            } else {
                ts.colors.push_front((kmer_color, 1));
            }
    }

    fn join_structures<const REVERSE: bool>(
        dest: &mut Self::TempUnitigColorStructure,
        src: &Self::PartialUnitigsColorStructure,
        src_buffer: &<Self::PartialUnitigsColorStructure as SequenceExtraData>::TempBuffer,
        mut skip: u64,
    ) {
        let get_index = |i| {
            if REVERSE {
                src.slice.start + i
            } else {
                src.slice.end - i - 1
            }
        };

        let len = src.slice.end - src.slice.start;

        let colors_slice = &src_buffer.colors.as_slice();

        for i in 0..len {
            let color = colors_slice[get_index(i)];

            if color.1 <= skip {
                skip -= color.1;
            } else {
                let left = color.1 - skip;
                skip = 0;
                if dest.colors.back().map(|x| x.0 == color.0).unwrap_or(false) {
                    dest.colors.back_mut().unwrap().1 += left;
                } else {
                    dest.colors.push_back((color.0, left));
                }
            }
        }
    }

    fn pop_base(target: &mut Self::TempUnitigColorStructure) {
        if let Some(last) = target.colors.back_mut() {
            last.1 -= 1;
            if last.1 == 0 {
                target.colors.pop_back();
            }
        }
    }

    fn encode_part_unitigs_colors(
        ts: &mut Self::TempUnitigColorStructure,
        colors_buffer: &mut <Self::PartialUnitigsColorStructure as SequenceExtraData>::TempBuffer,
    ) -> Self::PartialUnitigsColorStructure {
        colors_buffer.colors.clear();
        colors_buffer.colors.extend(ts.colors.iter());

        UnitigColorDataSerializer {
            slice: 0..colors_buffer.colors.len(),
        }
    }

    fn print_color_data(
        colors: &Self::PartialUnitigsColorStructure,
        colors_buffer: &<Self::PartialUnitigsColorStructure as SequenceExtraData>::TempBuffer,
        buffer: &mut impl Write,
    ) {
        for i in colors.slice.clone() {
            write!(
                buffer,
                " C:{:x}:{}",
                colors_buffer.colors[i].0, colors_buffer.colors[i].1
            )
            .unwrap();
        }
    }

    fn debug_tucs(str: &Self::TempUnitigColorStructure, seq: &[u8]) {
        let sum: u64 = str.colors.iter().map(|x| x.1).sum::<u64>() + 62;
        if sum as usize != seq.len() {
            println!("Temp values: {} {}", sum as usize, seq.len());
            println!("Dbg: {:?}", str.colors);
            assert_eq!(sum as usize, seq.len());
        }
    }
}

#[derive(Debug)]
pub struct DefaultUnitigsTempColorData {
    colors: VecDeque<(ColorIndexType, u64)>,
}

#[derive(Debug)]
pub struct UnitigsSerializerTempBuffer {
    colors: Vec<(ColorIndexType, u64)>,
}

#[derive(Clone, Debug)]
pub struct UnitigColorDataSerializer {
    slice: Range<usize>,
}

impl SequenceExtraDataTempBufferManagement<UnitigsSerializerTempBuffer>
    for UnitigColorDataSerializer
{
    fn new_temp_buffer() -> UnitigsSerializerTempBuffer {
        UnitigsSerializerTempBuffer { colors: Vec::new() }
    }

    fn clear_temp_buffer(buffer: &mut UnitigsSerializerTempBuffer) {
        buffer.colors.clear();
    }

    fn copy_temp_buffer(dest: &mut UnitigsSerializerTempBuffer, src: &UnitigsSerializerTempBuffer) {
        dest.colors.clear();
        dest.colors.extend_from_slice(&src.colors);
    }

    fn copy_extra_from(
        extra: Self,
        src: &UnitigsSerializerTempBuffer,
        dst: &mut UnitigsSerializerTempBuffer,
    ) -> Self {
        let start = dst.colors.len();
        dst.colors.extend(&src.colors[extra.slice]);
        Self {
            slice: start..dst.colors.len(),
        }
    }
}

impl SequenceExtraData for UnitigColorDataSerializer {
    type TempBuffer = UnitigsSerializerTempBuffer;

    fn decode_extended(buffer: &mut Self::TempBuffer, reader: &mut impl Read) -> Option<Self> {
        let start = buffer.colors.len();

        let colors_count = decode_varint(|| reader.read_u8().ok())?;

        for _ in 0..colors_count {
            buffer.colors.push((
                decode_varint(|| reader.read_u8().ok())? as ColorIndexType,
                decode_varint(|| reader.read_u8().ok())?,
            ));
        }
        Some(Self {
            slice: start..buffer.colors.len(),
        })
    }

    fn encode_extended(&self, buffer: &Self::TempBuffer, writer: &mut impl Write) {
        let colors_count = self.slice.end - self.slice.start;
        encode_varint(|b| writer.write_all(b), colors_count as u64).unwrap();

        for i in self.slice.clone() {
            let el = buffer.colors[i];
            encode_varint(|b| writer.write_all(b), el.0 as u64).unwrap();
            encode_varint(|b| writer.write_all(b), el.1).unwrap();
        }
    }

    #[inline(always)]
    fn max_size(&self) -> usize {
        (2 * (self.slice.end - self.slice.start) + 1) * VARINT_MAX_SIZE
    }
}
