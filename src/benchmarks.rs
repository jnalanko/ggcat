pub fn add_two(a: i32) -> i32 {
    a + 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressed_read::CompressedRead;
    use crate::hash::{HashFunction, HashFunctionFactory};
    use crate::hashes::fw_nthash::{ForwardNtHashIterator, ForwardNtHashIteratorFactory};
    use crate::hashes::fw_seqhash::{ForwardSeqHashFactory, ForwardSeqHashIterator};
    use crate::rolling_minqueue::RollingMinQueue;
    use crate::utils::Utils;
    use crate::varint::encode_varint;
    use bincode::DefaultOptions;
    use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
    use serde::{Deserialize, Serialize};
    use std::fs::File;
    use std::io::{BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
    use std::ops::Deref;
    use test::Bencher;

    #[test]
    fn it_works() {
        assert_ne!(4, add_two(2));
    }

    const TEST_SIZE: usize = 10000000;

    type VecType = u8;

    #[bench]
    fn bench_loop_vec(b: &mut Bencher) {
        let mut vec = Vec::with_capacity(TEST_SIZE);
        for i in 0..TEST_SIZE {
            vec.push(i as VecType);
        }
        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            for i in 0..TEST_SIZE {
                sum += vec[i] as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_loop_optimized(b: &mut Bencher) {
        let mut vec = Vec::with_capacity(TEST_SIZE);
        for i in 0..TEST_SIZE {
            vec.push(i as VecType);
        }
        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            let ptr = vec.as_ptr();
            unsafe {
                for i in 0..TEST_SIZE {
                    sum += (*ptr.add(i)) as usize;
                }
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    // #[link(name = "test")]
    extern "C" {
        fn compute(data: *const u64, len: usize) -> usize;
    }

    #[bench]
    fn bench_iter_vec(b: &mut Bencher) {
        let mut vec = Vec::with_capacity(TEST_SIZE);
        for i in 0..TEST_SIZE {
            vec.push(i as VecType);
        }
        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            for x in vec.iter() {
                sum += *x as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    // #[bench]
    // fn bench_c_program(b: &mut Bencher) {
    //     let mut vec = Vec::with_capacity(TEST_SIZE);
    //     for i in 0..TEST_SIZE {
    //         vec.push(i as VecType);
    //     }
    //     let mut sum = 0;
    //
    //     b.iter(|| unsafe {
    //         sum = 0;
    //         sum = compute(vec.as_ptr(), vec.len());
    //     });
    //
    //     assert_ne!(sum, 49999995000000);
    // }

    #[bench]
    fn bench_cursor_vec(b: &mut Bencher) {
        let mut vec = Vec::with_capacity(TEST_SIZE);
        for i in 0..TEST_SIZE {
            vec.push(i as u8);
        }
        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            let mut cursor = Cursor::new(&vec);
            while let Ok(byte) = cursor.read_u8() {
                sum += byte as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read_fast(b: &mut Bencher) {
        let mut file = File::open("/tmp/test").unwrap();

        let mut sum = 0;

        let mut vec = Vec::<u8>::new();
        vec.reserve(TEST_SIZE);

        b.iter(|| {
            sum = 0;
            vec.clear();
            file.seek(SeekFrom::Start(0));
            file.read_to_end(&mut vec);
            for &byte in vec.iter() {
                sum += byte as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read_fast_cursor(b: &mut Bencher) {
        let mut file = File::open("/tmp/test").unwrap();

        let mut sum = 0;

        let mut vec = Vec::<u8>::new();
        vec.reserve(TEST_SIZE);

        b.iter(|| {
            sum = 0;
            vec.clear();
            file.seek(SeekFrom::Start(0));
            file.read_to_end(&mut vec);
            let mut cursor = Cursor::new(&vec);
            while let Ok(byte) = cursor.read_u8() {
                sum += byte as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read_fast_cursor_bytes_slice(b: &mut Bencher) {
        let mut file = File::open("/tmp/test").unwrap();

        let mut sum = 0;

        let mut vec = Vec::<u8>::new();
        vec.reserve(TEST_SIZE);

        b.iter(|| {
            sum = 0;
            vec.clear();
            file.seek(SeekFrom::Start(0));
            file.read_to_end(&mut vec);
            let mut cursor = Cursor::new(vec.as_slice());
            for byte in cursor.bytes() {
                sum += byte.unwrap() as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read_fast_cursor_bytes(b: &mut Bencher) {
        let mut file = File::open("/tmp/test").unwrap();

        let mut sum = 0;

        let mut vec = Vec::<u8>::new();
        vec.reserve(TEST_SIZE);

        b.iter(|| {
            sum = 0;
            vec.clear();
            file.seek(SeekFrom::Start(0));
            file.read_to_end(&mut vec);
            let mut cursor = Cursor::new(vec.as_slice());
            for byte in cursor.bytes() {
                sum += byte.unwrap() as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read(b: &mut Bencher) {
        let mut file = File::open("/tmp/test").unwrap();

        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            file.seek(SeekFrom::Start(0));
            let mut buffer = BufReader::with_capacity(TEST_SIZE, &mut file);
            while let Ok(byte) = buffer.read_u8() {
                sum += byte as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read_fast2(b: &mut Bencher) {
        let mut file = File::open("/tmp/test").unwrap();

        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            file.seek(SeekFrom::Start(0));
            let mut buffer = BufReader::with_capacity(TEST_SIZE, &mut file);

            let mut data = [0; TEST_SIZE / 10];
            while let Ok(()) = buffer.read_exact(&mut data[..]) {
                for &x in data.iter() {
                    sum += x as usize;
                }
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read_fast_cursor_bytes_mmap(b: &mut Bencher) {
        let mut file = filebuffer::FileBuffer::open("/tmp/test").unwrap();

        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            let mut cursor = Cursor::new(&file);
            for byte in cursor.bytes() {
                sum += byte.unwrap() as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read_fast_cursor_bytes_mmap_slice(b: &mut Bencher) {
        let mut file = filebuffer::FileBuffer::open("/tmp/test").unwrap();

        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            let mut cursor = Cursor::new(file.deref());
            while let Ok(byte) = cursor.read_u8() {
                sum += byte as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[bench]
    fn bench_file_read_fast_mmap(b: &mut Bencher) {
        let mut file = filebuffer::FileBuffer::open("/tmp/test").unwrap();

        let mut sum = 0;

        b.iter(|| {
            sum = 0;
            // let mut cursor = Cursor::new(&file);
            for &byte in file.iter() {
                sum += byte as usize;
            }
        });

        assert_ne!(sum, 49999995000000);
    }

    #[derive(Serialize, Deserialize)]
    struct Test {
        #[serde(with = "crate::varint")]
        x: u64,
        #[serde(with = "crate::varint")]
        y: u64,
    }

    #[bench]
    fn bench_varint_encoding(b: &mut Bencher) {
        const TEST_SIZE: usize = 10000000;

        let mut test_vec = Vec::with_capacity(TEST_SIZE);

        for i in 0..TEST_SIZE as u64 {
            test_vec.push(Test {
                x: i,
                y: i + 1230120312031023,
            })
        }

        let mut ser_vec = Vec::with_capacity(TEST_SIZE * 18);

        b.iter(|| {
            ser_vec.clear();
            bincode::serialize_into(&mut ser_vec, &test_vec).unwrap();
        });
        println!("Size {}", ser_vec.len());
    }

    #[bench]
    fn bench_varint_encoding_custom(b: &mut Bencher) {
        const TEST_SIZE: usize = 10000000;

        let mut test_vec = Vec::with_capacity(TEST_SIZE);

        for i in 0..TEST_SIZE as u64 {
            test_vec.push(Test {
                x: i,
                y: i + 1230120312031023,
            })
        }

        let mut ser_vec = Vec::with_capacity(TEST_SIZE * 18);

        b.iter(|| {
            ser_vec.clear();
            for test in test_vec.iter() {
                encode_varint(|b| ser_vec.write_all(b), test.x as u64);
                encode_varint(|b| ser_vec.write_all(b), test.y as u64);
                // ser_vec.write_u64::<LittleEndian>(test.x as u64);
                // ser_vec.write_u64::<LittleEndian>(test.y as u64);
            }
        });
        println!("Size {}", ser_vec.len());
    }

    #[test]
    fn test_nthash() {
        // TGGATGGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATGGG ==> TATGTATATATATATATATATATATATATATATATATATATATATATATATATATATATGTGT
        // let str0 = b"GNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNG";
        // let str1 = b"TNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNT";

        // let str0 = b"GATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATTT";
        // let str1 = b"TATATATATATATATATATATATATATATATATATATATATATATATATATATATATATTG";

        // let str0 = b"NGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNN";
        // let str1 = b"NTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNN";

        let str0 = b"TAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let str1 = b"TAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAT";

        let h = 6116442737687281716u64;

        let hash = ForwardNtHashIterator::new(&str0[..], 15).unwrap();
        println!("{:?}", hash.iter().collect::<Vec<_>>());
        let hash1 = ForwardNtHashIterator::new(&str1[..], 15).unwrap();
        println!("{:?}", hash1.iter().collect::<Vec<_>>())
    }

    #[test]
    fn test_seqhash() {
        // TGGATGGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATGGG ==> TATGTATATATATATATATATATATATATATATATATATATATATATATATATATATATGTGT
        // let str0 = b"GNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNG";
        // let str1 = b"TNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNT";

        // let str0 = b"GATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATAGATTT";
        // let str1 = b"TATATATATATATATATATATATATATATATATATATATATATATATATATATATATATTG";

        // let str0 = b"NGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNNGNNN";
        // let str1 = b"NTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNNTNNN";

        // let str0 = b"TTTCTTTTTTTTTTTTTTTAATTTTGAGACAA";
        // let str1 = b"TTTCTTTTTTTTTTTTTTTAATTTTGCCCCAATTTCTTTTTTTTTTTTTTTAATTTTGAGACAA";
        //
        // let hash = SeqHashIterator::new(&str0[..], 32).unwrap();
        // println!("{:?}", hash.iter().collect::<Vec<_>>());
        // let hash1 = SeqHashIterator::new(&str1[..], 32).unwrap();
        // println!("{:?}", hash1.iter().collect::<Vec<_>>());

        let a: Vec<_> = (&b"GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAC"[..])
            .iter()
            .map(|x| Utils::compress_base(*x))
            .collect();
        let b: Vec<_> = (&b"CACCCACCCATTCACCTATCCATCCATCCAACCGTCCATCTGTTCATTC"[..])
            .iter()
            .map(|x| Utils::compress_base(*x))
            .collect();

        let ha: Vec<_> = ForwardNtHashIterator::new(&a[..], 15)
            .unwrap()
            .iter()
            .collect();
        // let hb: Vec<_> = SeqHashIterator::new(&b[..], 31).unwrap().iter().collect();

        // let hc = SeqHashFactory::manual_roll_forward(ha, 32, a[0], *b.last().unwrap());

        println!("X {:?}", ha);
        // println!("Y {:?}", hb);
        // println!("{:b}", hc);
    }

    #[test]
    fn test_minqueue() {
        let vec = vec![0x23, 0x47, 0xFA, 0x7D, 0x59, 0xFF, 0xA1, 0x84];

        // let hashes = ::new(&seq[..], context.m);
        // let mut minimizer_queue = RollingMinQueue::<NtHashIteratorFactory>::new(32 - 15);

        // let mut rolling_iter = minimizer_queue.make_iter(hashes.iter());
    }

    #[test]
    fn test_comprread() {
        let vec = vec![0x23, 0x47, 0xFA, 0x7D, 0x59, 0xFF, 0xA1, 0x84];

        let read1 = CompressedRead::from_compressed_reads(&vec[..], 0, 32).sub_slice(1..32);
        let read12 = CompressedRead::from_compressed_reads(&vec[..], 1, 31).sub_slice(0..31);
        println!("{}", read1.to_string());
        println!("{}", read12.to_string());
    }
}