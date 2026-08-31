use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Tell cargo to rerun if these files change
    println!("cargo:rerun-if-changed=data/resources/window.blp");
    println!("cargo:rerun-if-changed=data/resources/gresource.xml");

    // Compile Blueprint to XML
    let blueprint_input = PathBuf::from("data/resources/window.blp");
    let blueprint_output = PathBuf::from("data/resources/window.ui");

    let status = Command::new("blueprint-compiler")
        .args(["compile", "--output", blueprint_output.to_str().unwrap(), blueprint_input.to_str().unwrap()])
        .status()
        .expect("Failed to run blueprint-compiler. Is it installed?");

    if !status.success() {
        panic!("blueprint-compiler failed with status: {}", status);
    }

    // Compile GResource
    let gresource_xml = PathBuf::from("data/resources/gresource.xml");
    let gresource_output = PathBuf::from("data/resources/org.example.DocumentSearcher.gresource");

    let status = Command::new("glib-compile-resources")
        .args([
            "--target", gresource_output.to_str().unwrap(),
            "--sourcedir", "data/resources",
            gresource_xml.to_str().unwrap(),
        ])
        .status()
        .expect("Failed to run glib-compile-resources. Is it installed?");

    if !status.success() {
        panic!("glib-compile-resources failed with status: {}", status);
    }

    // Generate Rust code for GResource
    gresource::generate(&gresource_xml, "data/resources", None).expect("Failed to generate GResource Rust code");
}