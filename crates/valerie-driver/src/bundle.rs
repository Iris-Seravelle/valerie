//! Production distribution bundler for Valerie.
//!
//! Generates standalone distribution packages (.s, .o, .h, and .a)
//! ready for immediate drop-in integration into C/Rust/Go projects.

use crate::driver::{BackendMode, DriverError, assemble_string_with_backend};
use crate::header::generate_c_header;
use crate::vale_bridge::ValeProcedure;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use valerie_target::TargetDescriptor;

#[derive(Debug)]
pub struct BundleArtifacts {
    pub asm_path: PathBuf,
    pub obj_path: PathBuf,
    pub header_path: PathBuf,
    pub archive_path: PathBuf,
}

/// Bundles verified procedures into a production distribution directory.
pub fn bundle_distribution(
    base_name: &str,
    procs: &[ValeProcedure],
    out_dir: &Path,
    target: &TargetDescriptor,
    backend_mode: BackendMode,
) -> Result<BundleArtifacts, DriverError> {
    fs::create_dir_all(out_dir)?;

    let asm_path = out_dir.join(format!("{}.s", base_name));
    let obj_path = out_dir.join(format!("{}.o", base_name));
    let header_path = out_dir.join(format!("{}.h", base_name));
    let archive_path = out_dir.join(format!("lib{}.a", base_name));

    // 1. Emit 1:1 assembly
    let mut asm_text = String::new();
    asm_text.push_str("    .text\n    .p2align 2\n\n");
    for proc in procs {
        asm_text.push_str(&format!(
            "    .global {}\n    .type {}, %function\n{}:\n",
            proc.name, proc.name, proc.name
        ));
        for inst in &proc.instructions {
            asm_text.push_str(&format!("    {}\n", inst));
        }
        asm_text.push_str("    ret\n\n");
    }
    asm_text.push_str("    .section .note.GNU-stack,\"\",@progbits\n");
    fs::write(&asm_path, &asm_text)?;

    // 2. Compile to object file (.o)
    assemble_string_with_backend(&asm_text, &obj_path, target, backend_mode)?;

    // 3. Generate matching C header (.h)
    let header_text = generate_c_header(base_name, procs);
    fs::write(&header_path, header_text)?;

    // 4. Create static library archive (.a) using `ar rcs`
    let mut ar_cmd = Command::new("ar");
    ar_cmd.arg("rcs").arg(&archive_path).arg(&obj_path);
    let ar_out = ar_cmd.output()?;
    if !ar_out.status.success() {
        return Err(DriverError::LinkerFailed {
            status: ar_out.status,
            stderr: String::from_utf8_lossy(&ar_out.stderr).to_string(),
        });
    }

    Ok(BundleArtifacts {
        asm_path,
        obj_path,
        header_path,
        archive_path,
    })
}
