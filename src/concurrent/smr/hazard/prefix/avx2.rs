use crate::concurrent::smr::hazard::prefix::BePacked;
use crate::concurrent::smr::hazard::prefix::LePacked;

impl BePacked {
    pub(super) fn is_conflict_avx2(self, hazards: &[Self; 4]) -> bool {
        use core::arch::x86_64::*;
        validate!(self.is_node() ^ self.is_value());

        unsafe {
            // set hazard and broadcast prefix
            let h = _mm256_setr_epi64x(
                hazards[0].into_raw() as i64,
                hazards[1].into_raw() as i64,
                hazards[2].into_raw() as i64,
                hazards[3].into_raw() as i64,
            );
            let p = _mm256_set1_epi64x(self.into_raw() as i64);

            let zeros = _mm256_setzero_si256();
            let ones = _mm256_set1_epi64x(-1);

            // Case: `hazard` doesn't protect node or value
            // (h & p) & (0b11)
            let type_bits = _mm256_and_si256(_mm256_and_si256(h, p), _mm256_set1_epi64x(0b11));
            // != 0
            let type_match = _mm256_cmpgt_epi64(type_bits, zeros);

            // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
            // get overlap bit
            let h_no_overlap =
                _mm256_cmpeq_epi64(_mm256_and_si256(h, _mm256_set1_epi64x(0b100)), zeros);

            // fetch len
            let h_bits = _mm256_and_si256(h, _mm256_set1_epi64x(0b111_000));
            let p_bits = _mm256_and_si256(p, _mm256_set1_epi64x(0b111_000));

            // !h.overlap() && h.len > p.len
            let skip = _mm256_and_si256(h_no_overlap, _mm256_cmpgt_epi64(h_bits, p_bits));

            // Case: Overlapping prefix
            // h ^ p
            let xor = _mm256_xor_si256(h, p);

            let bits = _mm256_min_epu8(h_bits, p_bits);

            // Be::extract logic
            let prefix_mask = _mm256_srlv_epi64(ones, bits);

            let overlap = _mm256_cmpeq_epi64(_mm256_andnot_si256(prefix_mask, xor), zeros);

            // combine: type_match & !skip & overlap
            _mm256_testz_si256(type_match, _mm256_andnot_si256(skip, overlap)) == 0
        }
    }
}

impl LePacked {
    pub(super) fn is_conflict_avx2(self, hazards: &[Self; 4]) -> bool {
        use core::arch::x86_64::*;
        validate!(self.is_node() ^ self.is_value());

        unsafe {
            // set hazard and broadcast prefix
            let h = _mm256_setr_epi64x(
                hazards[0].into_raw() as i64,
                hazards[1].into_raw() as i64,
                hazards[2].into_raw() as i64,
                hazards[3].into_raw() as i64,
            );
            let p = _mm256_set1_epi64x(self.into_raw() as i64);

            let zeros = _mm256_setzero_si256();

            // Case: `hazard` doesn't protect node or value
            // (h & p) & (0b11 << 56)
            let type_bits =
                _mm256_and_si256(_mm256_and_si256(h, p), _mm256_set1_epi64x(0b11 << 56));
            // != 0
            let type_match = _mm256_cmpgt_epi64(type_bits, zeros);

            // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
            // get overlap bit
            let h_no_overlap =
                _mm256_cmpeq_epi64(_mm256_and_si256(h, _mm256_set1_epi64x(0b100 << 56)), zeros);

            // fetch len
            let h_bits = _mm256_and_si256(h, _mm256_set1_epi64x(0b111_000 << 56));
            let p_bits = _mm256_and_si256(p, _mm256_set1_epi64x(0b111_000 << 56));

            // !h.overlap() && h.len > p.len
            let skip = _mm256_and_si256(h_no_overlap, _mm256_cmpgt_epi64(h_bits, p_bits));

            // Case: Overlapping prefix
            // h ^ p
            let xor = _mm256_xor_si256(h, p);

            let bits = _mm256_srli_epi64::<56>(_mm256_min_epu8(h_bits, p_bits));

            // Be::extract logic
            let one = _mm256_set1_epi64x(1);
            let prefix_mask = _mm256_sub_epi64(_mm256_sllv_epi64(one, bits), one);

            let overlap = _mm256_cmpeq_epi64(_mm256_and_si256(xor, prefix_mask), zeros);

            // combine: type_match & !skip & overlap
            _mm256_testz_si256(type_match, _mm256_andnot_si256(skip, overlap)) == 0
        }
    }
}
