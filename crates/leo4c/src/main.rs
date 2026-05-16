//! leo4c — CLI for leo4 IDL.
//!
//!   leo4c parse     <file>   — print parsed AST (`Debug`)
//!   leo4c canonical <file>   — print canonical (collapsed) IDL form
//!                              that feeds the schema-hash input
//!   leo4c mangle    <file>   — emit the mangling table as JSON
//!                              matching `<pkg>.leo4-mangling`

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use leo4_idl::{mangle, mangle_type, parse, render_canonical, Hash};

#[derive(Parser)]
#[command(name = "leo4c", version, about = "leo4 IDL toolkit")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the parsed AST.
    Parse {
        file: PathBuf,
    },
    /// Print the canonical (collapsed) form used as the schema-hash input.
    Canonical {
        file: PathBuf,
    },
    /// Print the mangling table as JSON matching `<pkg>.leo4-mangling`.
    Mangle {
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("leo4c: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(cmd: Cmd) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Cmd::Parse { file } => {
            let text = std::fs::read_to_string(file)?;
            let schema = parse(&text)?;
            println!("{schema:#?}");
        }
        Cmd::Canonical { file } => {
            let text = std::fs::read_to_string(file)?;
            let schema = parse(&text)?;
            let canonical = render_canonical(
                &schema.package,
                &schema.interface,
                &schema.user_decls,
                &schema.funcs,
                /* pretty */ false,
            );
            println!("{canonical}");
        }
        Cmd::Mangle { file } => {
            let text = std::fs::read_to_string(file)?;
            let schema = parse(&text)?;
            let canonical = render_canonical(
                &schema.package,
                &schema.interface,
                &schema.user_decls,
                &schema.funcs,
                false,
            );
            let hash = Hash::of_str(&canonical);

            // Group func decls by name so multiple monomorphisations of
            // the same logical function end up in one mangling entry.
            let mut entries_by_name: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
            for f in &schema.funcs {
                let param_types: Vec<_> = f.params.iter().map(|(_, t)| t.clone()).collect();
                let param_types_json: Vec<_> = param_types
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "encoded": mangle_type(t),
                            // No generic-arg origin info on the Rust side
                            // (the monomorphised schema does not record it);
                            // leave `uses_generics` empty for now.
                            "uses_generics": serde_json::Value::Array(vec![]),
                        })
                    })
                    .collect();
                let mangled = mangle(
                    &schema.package,
                    &schema.interface,
                    &f.name,
                    &param_types,
                    hash,
                );
                let row = serde_json::json!({
                    "generic_args": serde_json::Value::Array(vec![]),
                    "param_types": param_types_json,
                    "mangled": mangled,
                });
                if let Some(slot) = entries_by_name.iter_mut().find(|(n, _)| n == &f.name) {
                    slot.1.push(row);
                } else {
                    entries_by_name.push((f.name.clone(), vec![row]));
                }
            }
            let mut entries = Vec::with_capacity(entries_by_name.len());
            for (name, insts) in entries_by_name {
                entries.push(serde_json::json!({
                    "logical_name": format!("{}::{}", schema.interface, name),
                    "generics": serde_json::Value::Array(vec![]),
                    "instantiations": insts,
                }));
            }
            let table = serde_json::json!({
                "version": 1,
                "package": schema.package,
                "schema_hash": hash.to_base32lc(),
                "entries": entries,
            });
            println!("{}", serde_json::to_string_pretty(&table)?);
        }
    }
    Ok(())
}
