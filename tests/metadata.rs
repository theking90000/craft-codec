//! Independent range model, persistence validation and integer boundaries.
use craft_codec::{
    Compression, Config, Encryption, Error, FORMAT_VERSION, FrameSizes, Framing, Lengths, Metadata,
    MetadataParts,
};

fn config(framing: Framing, compression: Compression, encryption: Encryption) -> Config {
    Config::new(framing, compression, encryption).unwrap()
}

#[test]
fn index_widths_and_length_minus_one_boundaries() {
    for (max, width) in [
        (65_535, 2),
        (65_536, 2),
        (65_537, 4),
        (1 << 32, 4),
        ((1 << 32) + 1, 8),
        (u64::MAX, 8),
    ] {
        let mut lengths = Lengths::new(max).unwrap();
        assert_eq!(lengths.width(), width);
        lengths.push(max).unwrap();
        assert_eq!(lengths.get(0), Some(max));
        assert_eq!(lengths.get(1), None);
        assert!(lengths.push(0).is_err());
        assert_eq!(lengths.len(), 1);
    }
    let mut lengths = Lengths::new(65_536).unwrap();
    lengths.push(65_536).unwrap();
    assert_eq!(lengths, Lengths::U16(vec![u16::MAX]));
    assert_eq!(lengths.push(65_537), Err(Error::Overflow));
    assert_eq!(Lengths::U64(vec![u64::MAX]).get(0), None);
}

#[test]
fn every_logical_range_matches_an_independent_boundary_model() {
    for framing in [Framing::Fixed(7), Framing::Variable(7)] {
        for compression in [Compression::None, Compression::Lz4] {
            for encryption in [Encryption::None, Encryption::Aes256Gcm] {
                let raw = match framing {
                    Framing::Fixed(_) => vec![7, 7, 7, 3],
                    _ => vec![3, 7, 1, 4],
                };
                let payload: Vec<u64> = raw
                    .iter()
                    .map(|&n| {
                        if compression == Compression::None {
                            n
                        } else {
                            (n / 2).max(1)
                        }
                    })
                    .collect();
                let mut metadata = Metadata::new(config(framing, compression, encryption));
                for (&raw, &payload) in raw.iter().zip(&payload) {
                    metadata
                        .push(FrameSizes::new(raw, payload).unwrap())
                        .unwrap();
                }
                let mut logical = vec![0];
                let mut physical = vec![0];
                for (&raw, &payload) in raw.iter().zip(&payload) {
                    logical.push(logical.last().unwrap() + raw);
                    physical.push(physical.last().unwrap() + payload + encryption.tag_len() as u64);
                }
                assert_eq!(metadata.logical_len(), *logical.last().unwrap());
                assert_eq!(metadata.stored_len(), *physical.last().unwrap());
                for start in 0..=metadata.logical_len() {
                    for end in start..=metadata.logical_len() {
                        let (query, mut frames) = metadata.range(start, Some(end)).unwrap();
                        let touched: Vec<usize> = (0..raw.len())
                            .filter(|&i| start < end && logical[i] < end && logical[i + 1] > start)
                            .collect();
                        assert_eq!(frames.remaining(), touched.len() as u64);
                        assert_eq!(frames.size_hint(), (touched.len(), Some(touched.len())));
                        if touched.is_empty() {
                            assert_eq!(query, 0..0);
                        } else {
                            assert_eq!(
                                query,
                                physical[touched[0]]..physical[touched[touched.len() - 1] + 1]
                            );
                        }
                        let mut selected = 0;
                        for i in touched {
                            let frame = frames.next().unwrap();
                            assert_eq!(frame.spec.index(), i as u64);
                            assert_eq!(frame.spec.raw_len(), raw[i] as usize);
                            assert_eq!(
                                frame.spec.stored_len(),
                                (physical[i + 1] - physical[i]) as usize
                            );
                            assert_eq!(
                                frame.selected,
                                (start.max(logical[i]) - logical[i]) as usize
                                    ..(end.min(logical[i + 1]) - logical[i]) as usize
                            );
                            selected += frame.selected.len();
                        }
                        assert_eq!(selected as u64, end - start);
                        assert!(frames.next().is_none());
                        assert!(frames.next().is_none());
                    }
                    assert_eq!(
                        metadata.range(start, None).unwrap().0,
                        metadata
                            .range(start, Some(metadata.logical_len()))
                            .unwrap()
                            .0
                    );
                }
                let restored = Metadata::from_parts(metadata.clone().into_parts()).unwrap();
                assert_eq!(restored, metadata);
            }
        }
    }
}

#[test]
fn documented_64k_example_includes_tag_and_preserves_slice() {
    let mut metadata = Metadata::new(config(
        Framing::Fixed(65_536),
        Compression::None,
        Encryption::Aes256Gcm,
    ));
    metadata
        .push(FrameSizes::new(65_536, 65_536).unwrap())
        .unwrap();
    let (query, mut frames) = metadata.range(50, Some(50_213)).unwrap();
    assert_eq!(query, 0..65_552);
    assert_eq!(frames.next().unwrap().selected, 50..50_213);
    assert!(frames.next().is_none());
}

#[test]
fn empty_invalid_and_short_final_frames() {
    let mut metadata = Metadata::new(config(
        Framing::Fixed(8),
        Compression::None,
        Encryption::None,
    ));
    assert!(metadata.range(0, None).unwrap().1.next().is_none());
    assert_eq!(metadata.last_frame_len(), None);
    assert!(metadata.range(1, None).is_err());
    metadata.push(FrameSizes::new(3, 3).unwrap()).unwrap();
    let before = metadata.clone();
    assert_eq!(
        metadata.push(FrameSizes::new(8, 8).unwrap()),
        Err(Error::FrameAfterFinal)
    );
    assert_eq!(metadata, before);
    assert!(metadata.range(2, Some(1)).is_err());
    assert!(metadata.range(0, Some(4)).is_err());
    assert_eq!(metadata.range(3, None).unwrap().0, 0..0);
    assert_eq!(metadata.last_frame_len(), Some(3));
    assert_eq!(metadata.range(1, None).unwrap().0, 0..3);
}

#[test]
fn malformed_persistence_is_rejected() {
    let cfg = config(Framing::Variable(100), Compression::Lz4, Encryption::None);
    let mut metadata = Metadata::new(cfg);
    metadata.push(FrameSizes::new(50, 25).unwrap()).unwrap();
    let base = metadata.into_parts();
    for variant in 0..9 {
        let mut parts = base.clone();
        match variant {
            0 => parts.version = FORMAT_VERSION + 1,
            1 => parts.frame_count = 2,
            2 => parts.last_frame_len = Some(50),
            3 => parts.raw_lengths = None,
            4 => parts.payload_lengths = None,
            5 => parts.raw_lengths = Some(Lengths::U32(vec![49])),
            6 => parts.raw_lengths = Some(Lengths::U16(vec![100])),
            7 => parts.payload_lengths = Some(Lengths::U16(vec![50])),
            8 => parts.raw_lengths = Some(Lengths::U64(vec![u64::MAX])),
            _ => unreachable!(),
        }
        assert!(Metadata::from_parts(parts).is_err(), "variant {variant}");
    }
}

#[test]
fn fixed_metadata_checks_overflow_without_allocating_frames() {
    let cfg = config(Framing::Fixed(8), Compression::None, Encryption::None);
    let mut parts = MetadataParts {
        version: FORMAT_VERSION,
        config: cfg,
        frame_count: u64::MAX,
        last_frame_len: Some(8),
        raw_lengths: None,
        payload_lengths: None,
    };
    assert_eq!(Metadata::from_parts(parts.clone()), Err(Error::Overflow));
    parts.frame_count = 1;
    parts.last_frame_len = Some(9);
    assert_eq!(
        Metadata::from_parts(parts.clone()),
        Err(Error::InvalidMetadata)
    );
    parts.last_frame_len = Some(8);
    parts.raw_lengths = Some(Lengths::U16(vec![7]));
    assert!(Metadata::from_parts(parts).is_err());
}

#[test]
fn large_fixed_ranges_need_no_frame_vector() {
    let cfg = config(Framing::Fixed(1), Compression::None, Encryption::None);
    let metadata = Metadata::from_parts(MetadataParts {
        version: FORMAT_VERSION,
        config: cfg,
        frame_count: u64::MAX,
        last_frame_len: Some(1),
        raw_lengths: None,
        payload_lengths: None,
    })
    .unwrap();
    let (query, mut frames) = metadata.range(u64::MAX - 2, None).unwrap();
    assert_eq!(query, u64::MAX - 2..u64::MAX);
    assert_eq!(frames.next().unwrap().spec.index(), u64::MAX - 2);
    assert_eq!(frames.next().unwrap().spec.index(), u64::MAX - 1);
    assert!(frames.next().is_none());
}
