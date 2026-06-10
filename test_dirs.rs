fn main() {
    let dirs = directories::ProjectDirs::from("com", "temidaradev", "kopuz").unwrap();
    println!("cache: {:?}", dirs.cache_dir());
}
