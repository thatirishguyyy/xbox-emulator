fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for f in args.chunks(2) {
        let src = std::fs::read_to_string(&f[0]).expect("read");
        let t = std::time::Instant::now();
        let out = tombox_xshader::branch_free::rewrite(&src);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let count = |s: &str, p: &str| s.matches(p).count();
        println!("{}: {ms:.1} ms  ?: {} -> {}  &&: {} -> {}  ||: {} -> {}", f[0], count(&src, "?"), count(&out, "?"), count(&src, "&&"), count(&out, "&&"), count(&src, "||"), count(&out, "||"));
        if let Some(o) = f.get(1) {
            std::fs::write(o, out).expect("write");
        }
    }
}
