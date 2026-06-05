use arctic::Ascend;
use arctic::concurrent;
use arctic::sequential;

#[test]
fn turso_range_24230c111c599daff93a7abc11c5c72b33d0ebfd() {
    // https://github.com/jennyhour/turso-arctic/blob/2c7cbf300adacf8482346d6f927753c9074d8bd7/core/mvcc/database/mod.rs#L88-L111
    const fn turso_row_id(row_id: i64) -> u128 {
        const SIGN: u64 = 1u64.rotate_right(1);

        let table_id = (-1i64) as u64 ^ SIGN;
        ((table_id as u128) << 64) | (((row_id as u64) ^ SIGN) as u128)
    }

    let mut map = sequential::Map::<u128, u64>::default();
    let entries = (0..10u64)
        .map(|index| (turso_row_id(index as i64), index))
        .collect::<Vec<_>>();

    for (row_id, index) in &entries {
        assert!(map.upsert(row_id, *index).is_ok());
    }

    for (row_id, index) in &entries {
        assert_eq!(map.get(row_id), Some(index));
    }

    let prefix = map.range(turso_row_id(5)..=turso_row_id(i64::MAX));
    let values = prefix.values::<Ascend>().copied().collect::<Vec<_>>();
    assert_eq!(values, (5..10).collect::<Vec<u64>>());
}

#[test]
fn insert_duplicate_82007770fb876db856313cebf12be21b9182f16a() {
    let map = concurrent::Map::<u64, u64>::default();
    map.insert(&0u64, 0u64).unwrap();
    map.insert(&0u64, 1u64).unwrap_err();
}

#[test]
fn range_common_prefix_72c2fceda258b00fc2e9d4a805b28e9ad8e8107d() {
    let map = crate::concurrent::Map::<u64, u64>::new();
    const NEEDLE: u64 = 0xE642_3BB1_ADBB_F000;
    const LOWER: u64 = 0x39_9100;
    const UPPER: u64 = 0xFF29_D24D_7E9A_920D;
    map.insert(&NEEDLE, 0).unwrap();
    map.range(LOWER..=UPPER)
        .entries::<crate::Ascend>()
        .for_each(|(key, value)| {
            assert_eq!(key, NEEDLE);
            assert_eq!(value, 0);
        })
}
