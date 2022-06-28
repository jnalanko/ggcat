use crate::assemble_pipeline::parallel_kmers_merge::structs::MapEntry;
use crate::colors::colors_manager::ColorsMergeManager;
use crate::colors::colors_memmap::ColorsMemMap;
use crate::config::DefaultColorsSerializer;
use crate::hashes::HashFunctionFactory;
use crate::io::concurrent::temp_reads::extra_data::{
    SequenceExtraData, SequenceExtraDataTempBufferManagement,
};
use crate::io::varint::{decode_varint, encode_varint, VARINT_MAX_SIZE};
use crate::ColorIndexType;
use byteorder::ReadBytesExt;
use core::slice::from_raw_parts;
use hashbrown::HashMap;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::ops::Range;
use std::path::Path;

pub struct MultipleColorsManager<H: HashFunctionFactory> {
    flags: Vec<(usize, usize, ColorIndexType)>,
    kmers: Vec<H::HashTypeUnextendable>,
    temp_colors: Vec<ColorIndexType>,
}

impl<H: HashFunctionFactory> ColorsMergeManager<H> for MultipleColorsManager<H> {
    type SingleKmerColorDataType = ColorIndexType;
    type GlobalColorsTable = ColorsMemMap<DefaultColorsSerializer>;

    fn create_colors_table(
        path: impl AsRef<Path>,
        color_names: Vec<String>,
    ) -> Self::GlobalColorsTable {
        ColorsMemMap::new(path, color_names)
    }

    fn print_color_stats(global_colors_table: &Self::GlobalColorsTable) {
        global_colors_table.print_stats();
    }

    type ColorsBufferTempStructure = Self;

    fn allocate_temp_buffer_structure() -> Self::ColorsBufferTempStructure {
        Self {
            flags: vec![],
            kmers: vec![],
            temp_colors: vec![],
        }
    }

    fn reinit_temp_buffer_structure(data: &mut Self::ColorsBufferTempStructure) {
        data.flags.clear();
        data.kmers.clear();
        data.temp_colors.clear()
    }

    fn add_temp_buffer_structure_el(
        data: &mut Self::ColorsBufferTempStructure,
        kmer_color: &ColorIndexType,
        el: (usize, H::HashTypeUnextendable),
        _entry: &mut MapEntry<Self::HashMapTempColorIndex>,
    ) {
        if data.flags.last().map(|l| &l.2) != Some(kmer_color) {
            if let Some(last) = data.flags.last_mut() {
                last.1 = data.kmers.len();
            }
            data.flags.push((data.kmers.len(), 0, kmer_color.clone()));
        }
        data.kmers.push(el.1);
    }

    type HashMapTempColorIndex = DefaultHashMapTempColorIndex;

    fn new_color_index() -> Self::HashMapTempColorIndex {
        DefaultHashMapTempColorIndex { temp_index: (0, 0) }
    }

    fn process_colors(
        global_colors_table: &Self::GlobalColorsTable,
        data: &mut Self::ColorsBufferTempStructure,
        map: &mut HashMap<H::HashTypeUnextendable, MapEntry<Self::HashMapTempColorIndex>>,
        min_multiplicity: usize,
    ) {
        let vec_len = data.kmers.len();
        data.flags.last_mut().iter_mut().for_each(|l| l.1 = vec_len);

        let mut last_partition = (0, 0);
        let mut last_color = 0;

        for (start, end, color) in &data.flags {
            for kmer_hash in &data.kmers[*start..*end] {
                let entry = map.get_mut(kmer_hash).unwrap();
                let entry_count = entry.get_counter();

                if entry_count < min_multiplicity {
                    continue;
                }

                unsafe {
                    if entry.color_index.temp_index.1 == 0 {
                        entry.color_index.temp_index.0 = data.temp_colors.len() as u32;
                        entry.color_index.temp_index.1 = entry_count as u32;
                        if data.temp_colors.capacity() < data.temp_colors.len() + entry_count {
                            data.temp_colors.reserve(entry_count);
                        }
                        data.temp_colors
                            .set_len(data.temp_colors.len() + entry_count);
                    }

                    data.temp_colors[entry.color_index.temp_index.0 as usize] = *color;
                    entry.color_index.temp_index.0 += 1;
                    entry.color_index.temp_index.1 -= 1;

                    // All colors were added, let's assign the final color
                    if entry.color_index.temp_index.1 == 0 {
                        let slice_start = entry.color_index.temp_index.0 as usize - entry_count;
                        let slice_end = entry.color_index.temp_index.0 as usize;

                        let slice = &mut data.temp_colors[slice_start..slice_end];
                        slice.sort_unstable();

                        // Assign the subset color index to the current kmer
                        let unique_colors = slice.partition_dedup().0;

                        let partition = from_raw_parts(unique_colors.as_ptr(), unique_colors.len());
                        if partition == &data.temp_colors[last_partition.0..last_partition.1] {
                            entry.color_index.color_index = last_color;
                        } else {
                            entry.color_index.color_index = global_colors_table.get_id(partition);
                            last_color = entry.color_index.color_index;
                            last_partition = (slice_start, slice_end);
                        }
                    }
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
        unsafe {
            if let Some(back_ts) = ts.colors.back_mut() {
                if back_ts.0 == entry.color_index.color_index {
                    back_ts.1 += 1;
                    return;
                }
            }
            ts.colors.push_back((entry.color_index.color_index, 1));
        }
    }

    fn extend_backward(
        ts: &mut Self::TempUnitigColorStructure,
        entry: &MapEntry<Self::HashMapTempColorIndex>,
    ) {
        unsafe {
            if let Some(front_ts) = ts.colors.front_mut() {
                if front_ts.0 == entry.color_index.color_index {
                    front_ts.1 += 1;
                    return;
                }
            }
            ts.colors.push_front((entry.color_index.color_index, 1));
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

pub union DefaultHashMapTempColorIndex {
    color_index: ColorIndexType,
    temp_index: (u32 /*Vector index*/, u32 /*Remaining*/),
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