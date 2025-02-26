#![feature(thread_local)]
#![feature(new_uninit)]
#![feature(slice_partition_dedup)]
#![feature(const_type_id)]
#![feature(int_roundings)]
#![feature(let_chains)]

use crate::storage::run_length::RunLengthColorsSerializer;

pub mod bundles;
pub mod colors_manager;
pub mod colors_memmap_writer;
pub mod managers;
pub mod non_colored;
pub mod parsers;
pub mod storage;

pub(crate) mod async_slice_queue;

pub type DefaultColorsSerializer = RunLengthColorsSerializer;
