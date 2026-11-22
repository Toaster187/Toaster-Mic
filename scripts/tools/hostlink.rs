//! Linker for the Windows host target (x86_64-pc-windows-gnu): NDK clang + lld
//! with the MinGW libraries shipped by rustup. GNU ld 2.44 mislinks the
//! raw-dylib import libraries of the `windows` crates; lld handles them.
//! Built by scripts/env.ps1: rustc -O hostlink.rs (paths baked in at build time).
use std::process::{exit, Command};

fn main() {
    let clang = env!("HOSTLINK_CLANG");
    let libs = env!("HOSTLINK_LIBS");
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Long command lines arrive as @response files.
    let mut text = args.join("\n");
    for a in args.iter().filter(|a| a.starts_with('@')) {
        text.push('\n');
        text.push_str(&std::fs::read_to_string(&a[1..]).unwrap_or_default());
    }
    let shared = text.lines().any(|l| l.trim_matches('"') == "-shared");
    let crt = format!("{libs}/{}", if shared { "dllcrt2.o" } else { "crt2.o" });
    let has_crt = text.contains("crt2.o");
    let mut cmd = Command::new(clang);
    cmd.args(["--target=x86_64-w64-windows-gnu", "-fuse-ld=lld", "-nostdlib", "-Wno-unused-command-line-argument"]);
    cmd.arg(format!("-L{libs}"));
    if !has_crt {
        cmd.arg(crt);
    }
    // Arguments may come in an @response file; clang understands those too.
    cmd.args(args.iter().filter(|a| a.as_str() != "-fno-use-linker-plugin"));
    match cmd.status() {
        Ok(s) => exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("hostlink: {clang}: {e}");
            exit(1)
        }
    }
}
