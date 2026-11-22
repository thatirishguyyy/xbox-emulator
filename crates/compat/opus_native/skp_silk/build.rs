include!("../opus_build.rs");

const EXPORTS: [&str; 8] = [
    "SKP_Silk_SDK_Get_Encoder_Size",
    "SKP_Silk_SDK_InitEncoder",
    "SKP_Silk_SDK_QueryEncoder",
    "SKP_Silk_SDK_Encode",
    "SKP_Silk_SDK_Get_Decoder_Size",
    "SKP_Silk_SDK_InitDecoder",
    "SKP_Silk_SDK_Decode",
    "SKP_Silk_SDK_get_version",
];

fn main() {
    let root = opus_root();
    rerun(&root);
    if !opus_available(&root) {
        println!("cargo:warning=Opus source tree not found at {}; skipping native build", root.display());
        return;
    }
    println!("cargo:rerun-if-changed=csrc/skp_silk.c");
    let mut b = opus_build(&root);
    b.files(opus_sources(&root, false));
    b.file("csrc/skp_silk.c");
    b.compile("tombox_skp_silk");
    export(&EXPORTS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
}
