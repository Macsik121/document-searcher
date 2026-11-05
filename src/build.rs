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
    println!("cargo:rerun-if-changed=data/resources/window.blp");
    println!("cargo:rerun-if-changed=data/resources/gresource.xml");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_DESKTOP");

    // The core/CLI build deliberately has no GTK toolchain requirement. This keeps
    // Windows development useful while the desktop build is assembled on Linux.
    if env::var_os("CARGO_FEATURE_DESKTOP").is_none() {
        return;
    }

    if !cfg!(target_os = "linux") {
        println!("cargo:warning=desktop resources are only built on Linux");
        return;
    }

    let blueprint_input = Path::new("data/resources/window.blp");
    let blueprint_output = Path::new("data/resources/window.ui");
    run_tool(
        "blueprint-compiler",
        &[
            "compile",
            "--output",
            blueprint_output.to_str().expect("non-UTF-8 output path"),
            blueprint_input.to_str().expect("non-UTF-8 input path"),
        ],
    );

    let gresource_xml = Path::new("data/resources/gresource.xml");
    let gresource_output = Path::new("data/resources/org.gtk.DocumentSearcher.gresource");
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
}
