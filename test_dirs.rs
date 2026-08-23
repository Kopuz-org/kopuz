fn main() {
    let dirs = directories::ProjectDirs::from("moe", "kopuz", "kopuz").unwrap();
    println!("cache: {:?}", dirs.cache_dir());
}
