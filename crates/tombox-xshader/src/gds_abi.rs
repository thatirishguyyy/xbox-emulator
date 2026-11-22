pub const TB_GDS_SPACE: u32 = 4_000_002;
pub const TB_GDS_REGISTER: u32 = 0;
pub const TB_GDS_BYTES: u64 = 0x1_0000;
pub const TB_GDS_TITLE_DWORDS: u32 = 0x3f00;
pub const TB_GDS_HLSL_DECL: &str = "RWByteAddressBuffer tb_gds : register(u0, space4000002);";
pub const TB_GDS_BYTES_PER_TITLE_DWORD: u32 = 4;
pub const TB_GDS_OA_TABLE_DWORD: u32 = TB_GDS_TITLE_DWORDS;
pub const TB_GDS_OA_COUNTERS: u32 = 8;
pub const TB_GDS_OA_STRIDE_DWORDS: u32 = 4;
pub const TB_GDS_OA_FIELD_COUNTER_DWORD: u32 = 0;
pub const TB_GDS_OA_FIELD_CNTL: u32 = 1;
pub const TB_GDS_OA_FIELD_STAGE: u32 = 2;
pub const TB_GDS_OA_FIELD_WAVE_RELEASE: u32 = 3;
pub const TB_GWS_TABLE_DWORD: u32 = TB_GDS_OA_TABLE_DWORD + TB_GDS_OA_COUNTERS * TB_GDS_OA_STRIDE_DWORDS;
pub const TB_GWS_RESOURCES: u32 = 64;
pub const TB_GWS_COUNTER_MASK: u32 = (1 << 12) - 1;

pub const fn tb_gds_byte_offset(title_dword: u32) -> u32 {
    title_dword * TB_GDS_BYTES_PER_TITLE_DWORD
}

pub const fn tb_gds_oa_byte_offset(counter: u32, field: u32) -> u32 {
    tb_gds_byte_offset(TB_GDS_OA_TABLE_DWORD + counter * TB_GDS_OA_STRIDE_DWORDS + field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_append_table_sits_after_title_gds() {
        assert_eq!(tb_gds_byte_offset(5), 20);
        assert_eq!(tb_gds_oa_byte_offset(0, TB_GDS_OA_FIELD_COUNTER_DWORD), TB_GDS_TITLE_DWORDS * 4);
        let end = tb_gds_oa_byte_offset(TB_GDS_OA_COUNTERS, 0) as u64;
        assert!(end <= TB_GDS_BYTES);
        assert_eq!(tb_gds_oa_byte_offset(1, TB_GDS_OA_FIELD_WAVE_RELEASE) - tb_gds_oa_byte_offset(0, 0), 4 * (TB_GDS_OA_STRIDE_DWORDS + 3));
    }

    #[test]
    fn hlsl_decl_matches_binding_constants() {
        assert_eq!(TB_GDS_HLSL_DECL, format!("RWByteAddressBuffer tb_gds : register(u{TB_GDS_REGISTER}, space{TB_GDS_SPACE});"));
        assert!(TB_GDS_TITLE_DWORDS as u64 * 4 <= TB_GDS_BYTES);
        assert_ne!(TB_GDS_SPACE, crate::hlsl::TB_MIP_SPACE);
        assert_ne!(TB_GDS_SPACE, crate::hlsl::TB_GMEM_SPACE);
    }
}
