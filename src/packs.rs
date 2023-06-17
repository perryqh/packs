mod configuration;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashSet;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::PathBuf;

mod cache;
pub(crate) mod checker;
pub mod cli;
mod inflector_shim;
mod pack_set;
pub mod package_todo;
pub mod parser;

// Re-exports: Eventually, these may be part of the public API for packs
pub use crate::packs::checker::Violation;
pub use crate::packs::pack_set::PackSet;
pub use configuration::Configuration;
pub use package_todo::PackageTodo;
pub use parser::ruby::packwerk::extractor::Range;
pub use parser::ruby::packwerk::extractor::UnresolvedReference;

use self::checker::ViolationIdentifier;

pub fn greet() {
    println!("👋 Hello! Welcome to packs 📦 🔥 🎉 🌈. This tool is under construction.")
}

pub fn list(configuration: Configuration) {
    for pack in configuration.pack_set.packs {
        println!("{}", pack.yml.display())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ProcessedFile {
    pub absolute_path: PathBuf,
    pub unresolved_references: Vec<UnresolvedReference>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SourceLocation {
    line: usize,
    column: usize,
}

#[derive(Debug, Deserialize)]
pub struct RawPack {
    #[serde(default)]
    dependencies: HashSet<String>,
    #[serde(default)]
    ignored_dependencies: HashSet<String>,
    #[serde(default = "default_enforce_dependencies")]
    enforce_dependencies: String,
}

fn default_enforce_dependencies() -> String {
    "false".to_string()
}

// Make an enum for the configuration of a checker, which can be either false, true, or strict:
#[derive(Debug, Default, PartialEq, Eq, Deserialize, Clone)]
enum CheckerSetting {
    #[default]
    False,
    True,
    Strict,
}

impl CheckerSetting {
    /// Returns `true` if the checker setting is [`False`].
    ///
    /// [`False`]: CheckerSetting::False
    #[must_use]
    fn is_false(&self) -> bool {
        matches!(self, Self::False)
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
pub struct Pack {
    #[serde(skip_deserializing)]
    yml: PathBuf,
    #[serde(skip_deserializing)]
    name: String,
    #[serde(skip_deserializing)]
    relative_path: PathBuf,
    #[serde(default)]
    // I want to see if checkers and such can add their own deserialization
    // behavior to Pack via a trait or something? That would make extension simpler!
    dependencies: HashSet<String>,
    ignored_dependencies: HashSet<String>,
    package_todo: PackageTodo,
    enforce_dependencies: CheckerSetting,
}

impl Hash for Pack {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Implement the hash function for your struct fields
        // Call the appropriate `hash` method on the `Hasher` to hash each field
        self.name.hash(state);
    }
}

impl Pack {
    pub fn all_violations(&self) -> Vec<ViolationIdentifier> {
        let mut violations = Vec::new();
        let violations_by_pack = &self.package_todo.violations_by_defining_pack;
        for (defining_pack_name, violation_groups) in violations_by_pack {
            for (constant_name, violation_group) in violation_groups {
                for violation_type in &violation_group.violation_types {
                    for file in &violation_group.files {
                        let identifier = ViolationIdentifier {
                            violation_type: violation_type.clone(),
                            file: file.clone(),
                            constant_name: constant_name.clone(),
                            referencing_pack_name: self.name.clone(),
                            defining_pack_name: defining_pack_name.clone(),
                        };

                        violations.push(identifier);
                    }
                }
            }
        }
        violations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_for_file() {
        let configuration = configuration::get(
            PathBuf::from("tests/fixtures/simple_app")
                .canonicalize()
                .expect("Could not canonicalize path")
                .as_path(),
        );
        let absolute_file_path = PathBuf::from(
            "tests/fixtures/simple_app/packs/foo/app/services/foo.rb",
        )
        .canonicalize()
        .expect("Could not canonicalize path");

        assert_eq!(
            Some(String::from("packs/foo")),
            configuration.pack_set.for_file(&absolute_file_path)
        )
    }
}
