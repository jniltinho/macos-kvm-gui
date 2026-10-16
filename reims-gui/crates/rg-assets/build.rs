// include_dir! does not tell cargo about the files it embeds on stable Rust.
fn main() {
    println!("cargo::rerun-if-changed=../../assets");
}
