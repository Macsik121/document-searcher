use std::env;
use std::path::Path;
use std::process::Command;

fn run_tool(program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {program}: {error}"));

    if !status.success() {
        panic!("{program} failed with status {status}");
    }
}

fn main() {
    println!("cargo:rerun-if-changed=data/resources/gresource.xml");
    println!("cargo:rerun-if-changed=data/resources/styles.css");
    println!("cargo:rerun-if-changed=data/org.gtk.DocumentSearcher.gschema.xml");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_DESKTOP");

    // The core/CLI and Windows desktop builds deliberately have no GTK toolchain
    // requirement. GLib tools only run when building the Linux GTK desktop feature.
    if env::var_os("CARGO_FEATURE_DESKTOP").is_none() {
        return;
    }

    if !cfg!(target_os = "linux") {
        panic!(
            "the desktop feature requires a Linux GTK/GLib toolchain; use --no-default-features or --features windows-desktop on Windows"
        );
    }

    let out_dir = env::var("OUT_DIR").expect("Cargo did not set OUT_DIR");
    let gresource_xml = Path::new("data/resources/gresource.xml");
    let gresource_output = Path::new(&out_dir).join("org.gtk.DocumentSearcher.gresource");
    run_tool(
        "glib-compile-resources",
        &[
            "--target",
            gresource_output.to_str().expect("non-UTF-8 output path"),
            "--sourcedir",
            "data/resources",
            gresource_xml.to_str().expect("non-UTF-8 resource path"),
        ],
    );
    println!(
        "cargo:rustc-env=DOCUMENT_SEARCHER_GRESOURCE={}",
        gresource_output.display()
    );

    let schema_output = Path::new(&out_dir).join("schemas");
    std::fs::create_dir_all(&schema_output).expect("could not create schema output directory");
    run_tool(
        "glib-compile-schemas",
        &[
            "--targetdir",
            schema_output
                .to_str()
                .expect("non-UTF-8 schema output path"),
            "data",
        ],
    );
    println!(
        "cargo:rustc-env=DOCUMENT_SEARCHER_SCHEMA_DIR={}",
        schema_output.display()
    );
}
