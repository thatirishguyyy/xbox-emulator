//! CLI: extract native RDNA2 ISA from an Xbox Series shader DXBC blob.
//!
//!
//! Prints the DXBC→XDAT→zlib→FSSM→RDNA2 chain it walked and a hex/word dump of the ISA.

use std::process::ExitCode;
use tombox_xshader::{extract, parse_root_signature, Stage};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <shader.dxbc> [--root-signature f] [--hlsl-out f] [--isa-out f] [--fssm-out f]", args[0]);
        return ExitCode::FAILURE;
    }
    let path = &args[1];
    let mut isa_out = None;
    let mut fssm_out = None;
    let mut hlsl_out = None;
    let mut root_signature = None;
    let mut infer_root = false;
    let mut i = 2;
    while i < args.len() {
        let flag = args[i].as_str();
        if flag == "--infer-root-signature" || flag == "--infer-root" {
            infer_root = true;
            i += 1;
            continue;
        }
        let output = match flag {
            "--isa-out" => &mut isa_out,
            "--fssm-out" => &mut fssm_out,
            "--hlsl-out" => &mut hlsl_out,
            "--root-signature" => &mut root_signature,
            _ => {
                eprintln!("unknown option {flag}");
                return ExitCode::FAILURE;
            }
        };
        let Some(value) = args.get(i + 1).filter(|value| !value.starts_with("--")) else {
            eprintln!("missing file argument for {flag}");
            return ExitCode::FAILURE;
        };
        *output = Some(value);
        i += 2;
    }

    let blob = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut ex = match extract(&blob) {
        Ok(ex) => ex,
        Err(e) => {
            eprintln!("extract failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(path) = root_signature {
        let signature = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| parse_root_signature(&bytes));
        match signature {
            Ok(signature) => ex.root_sig = Some(signature),
            Err(e) => {
                eprintln!("cannot load root signature {path}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if infer_root && ex.root_sig.is_none() {
        if let Ok(signature) = tombox_xshader::infer_root_signature(&ex) {
            ex.root_sig = Some(signature);
        }
    }

    let stage = match ex.stage {
        Stage::Pixel => "Pixel".into(),
        Stage::Vertex => "Vertex".into(),
        Stage::Geometry => "Geometry".into(),
        Stage::Hull => "Hull".into(),
        Stage::Domain => "Domain".into(),
        Stage::Compute => "Compute".into(),
        Stage::Mesh => "Mesh".into(),
        Stage::Amplification => "Amplification".into(),
        Stage::Unknown(x) => format!("Unknown({x})"),
    };

    println!("== {path} ==");
    println!(
        "stage={stage}  SM={}.{}  bitcode_stripped={}  entry={:?}",
        ex.shader_model.0, ex.shader_model.1, ex.bitcode_stripped, ex.entry
    );
    println!(
        "DXBC chunks: inputSig={} outputSig={} PSV0={} rootSig={}",
        ex.has_input_sig, ex.has_output_sig, ex.has_psv0, ex.has_root_sig
    );
    println!(
        "FSSM: {} bytes   ISA: {} bytes ({} dwords) @ FSSM+{:#x}",
        ex.fssm.len(),
        ex.isa.len(),
        ex.isa.len() / 4,
        ex.isa_offset_in_fssm
    );

    // Structured RDNA2 decode (length-accurate), self-validated by landing on s_endpgm.
    for (tag, sig) in [("in ", &ex.input_sig), ("out", &ex.output_sig)] {
        for e in sig.iter() {
            println!("  sig {tag}: {}{} reg={} mask={:#06b} sysval={} type={} rw={:#06b}", e.name, e.index, e.register, e.mask, e.sysval, e.comp_type, e.rw_mask);
        }
    }
    for r in &ex.resources {
        println!("  psv0 resource: type={} space={} reg={}..{}", r.res_type, r.space, r.lower, r.upper);
    }
    if let Some(rs) = &ex.root_sig {
        let source = root_signature.map_or("embedded", |path| path.as_str());
        println!("  rootsig v{} flags={:#x} staticSamplers={} source={source}", rs.version, rs.flags, rs.num_static_samplers);
        for (i, p) in rs.params.iter().enumerate() {
            println!("    param[{i}] vis={} {:?}", p.visibility, p.kind);
        }
    }
    println!("--- RDNA2 disassembly ---");
    let insns = tombox_xshader::rdna2::disassemble(&ex.isa);
    let mut consumed = 0usize;
    let mut ended = false;
    for ins in &insns {
        let raw: Vec<String> = ins.raw.iter().map(|w| format!("{w:08x}")).collect();
        use tombox_xshader::rdna2::opnd_str;
        let dsts: Vec<String> = ins.dst.iter().map(opnd_str).collect();
        let srcs: Vec<String> = ins.src.iter().map(opnd_str).collect();
        let mut ops = String::new();
        if !dsts.is_empty() {
            ops.push_str(&dsts.join(","));
        }
        if !srcs.is_empty() {
            if !ops.is_empty() {
                ops.push_str(", ");
            }
            ops.push_str(&srcs.join(", "));
        }
        let aux = match ins.enc {
            tombox_xshader::rdna2::Enc::Exp => format!(" [target={} en={:#06b}]", ins.aux.imm, ins.aux.mask),
            tombox_xshader::rdna2::Enc::Vinterp => format!(" [attr{}.{}]", ins.aux.imm, ins.aux.imm2),
            tombox_xshader::rdna2::Enc::Smem => format!(" [off={:#x}]", ins.aux.imm),
            tombox_xshader::rdna2::Enc::Sopp | tombox_xshader::rdna2::Enc::Sopk => format!(" [imm={}]", ins.aux.imm),
            _ => String::new(),
        };
        println!(
            "  {:#06x}: {:<20} {:<16} {} {}",
            ins.offset,
            raw.join(" "),
            ins.mnemonic,
            ops,
            aux
        );
        consumed += ins.dwords * 4;
        if ins.mnemonic == "s_endpgm" {
            ended = true;
        }
    }
    let total = ex.isa.len();
    println!(
        "decode: {} instrs, consumed {}/{} bytes, terminated_on_s_endpgm={} (aligned_landing={})",
        insns.len(),
        consumed,
        total,
        ended,
        ended && consumed <= total && (total - consumed) < 4
    );

    if let Some(f) = isa_out {
        if let Err(e) = std::fs::write(&f, &ex.isa) {
            eprintln!("cannot write {f}: {e}");
        } else {
            println!("wrote ISA -> {f}");
        }
    }
    if let Some(f) = fssm_out {
        if let Err(e) = std::fs::write(&f, &ex.fssm) {
            eprintln!("cannot write {f}: {e}");
        } else {
            println!("wrote FSSM -> {f}");
        }
    }
    if let Some(f) = hlsl_out {
        match tombox_xshader::hlsl::translate(&ex) {
            Ok(t) => {
                if let Err(e) = std::fs::write(&f, &t.hlsl) {
                    eprintln!("cannot write {f}: {e}");
                    return ExitCode::FAILURE;
                }
                println!("wrote HLSL ({}) -> {f}", t.profile);
            }
            Err(e) => {
                eprintln!("translate failed: {e}");
                if ex.root_sig.is_none() {
                    eprintln!("Supply the pipeline's root signature with --root-signature if the shader depends on it.");
                }
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
