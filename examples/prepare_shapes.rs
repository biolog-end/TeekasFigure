//! Prepare grayscale brushes while preserving original alpha.
#[path = "../src/io/shape_conversion.rs"]
mod shape_conversion;
fn main() {
    let source = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "raw_shapes".into());
    let destination = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "input_shapes".into());
    match shape_conversion::convert_folder(
        std::path::Path::new(&source),
        std::path::Path::new(&destination),
        128,
    ) {
        Ok(message) => println!("{message}"),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}
