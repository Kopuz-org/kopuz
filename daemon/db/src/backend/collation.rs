//! The `LOCALE` collation listings sort by; only pooled connections register it, so no schema may name it.

pub use utils::collation::compare;

pub const NAME: &str = "LOCALE";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_migration_names_the_collation() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for entry in std::fs::read_dir(&dir).expect("migrations dir") {
            let path = entry.expect("entry").path();
            let sql = std::fs::read_to_string(&path).expect("migration");
            assert!(
                !sql.to_uppercase().contains(&format!("COLLATE {NAME}")),
                "{} names the {NAME} collation, which only pooled connections register",
                path.display()
            );
        }
    }
}
