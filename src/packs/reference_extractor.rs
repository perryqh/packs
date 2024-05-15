use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use ruby_references::configuration::ExtraReferenceFieldsFn;
use tracing::debug;

use crate::packs::{
    get_experimental_constant_resolver, get_zeitwerk_constant_resolver,
    parsing::ruby::rails_utils, process_files_with_cache, ProcessedFile,
};

use super::{
    checker::reference::Reference, file_utils::expand_glob, Configuration,
    SourceLocation,
};

pub(crate) fn get_all_references(
    configuration: &Configuration,
    absolute_paths: &HashSet<PathBuf>,
) -> anyhow::Result<Vec<Reference>> {
    let cache = configuration.get_cache();

    debug!("Getting unresolved references (using cache if possible)");

    let (constant_resolver, processed_files_to_check) = if configuration
        .experimental_parser
    {
        // The experimental parser needs *all* processed files to get definitions
        let all_processed_files: Vec<ProcessedFile> = process_files_with_cache(
            &configuration.included_files,
            cache,
            configuration,
        )?;

        let constant_resolver = get_experimental_constant_resolver(
            &configuration.absolute_root,
            &all_processed_files,
            &configuration.ignored_definitions,
        );

        let processed_files_to_check = all_processed_files
            .into_iter()
            .filter(|processed_file| {
                absolute_paths.contains(&processed_file.absolute_path)
            })
            .collect();

        (constant_resolver, processed_files_to_check)
    } else {
        let processed_files: Vec<ProcessedFile> =
            process_files_with_cache(absolute_paths, cache, configuration)?;

        // The zeitwerk constant resolver doesn't look at processed files to get definitions
        let constant_resolver = get_zeitwerk_constant_resolver(
            &configuration.pack_set,
            &configuration.constant_resolver_configuration(),
        );

        (constant_resolver, processed_files)
    };

    debug!("Turning unresolved references into fully qualified references");
    let references: anyhow::Result<Vec<Reference>> = processed_files_to_check
        .par_iter()
        .try_fold(
            Vec::new,
            // Start with an empty vector for each thread
            |mut acc, processed_file| {
                // Try to fold results within a thread
                for unresolved_ref in &processed_file.unresolved_references {
                    let mut refs = Reference::from_unresolved_reference(
                        configuration,
                        constant_resolver.as_ref(),
                        unresolved_ref,
                        &processed_file.absolute_path,
                    )?;
                    acc.append(&mut refs); // Collect references, return error if any
                }
                Ok(acc)
            },
        )
        .try_reduce(
            Vec::new, // Start with an empty vector for the reduction
            |mut acc, mut vec| {
                // Try to reduce results across threads
                acc.append(&mut vec); // Combine vectors, no error expected here
                Ok(acc)
            },
        );
    debug!("Finished turning unresolved references into fully qualified references");

    references
}

struct PackageNames {
    names: Vec<String>,
}

impl PackageNames {
    fn new(config: &Configuration) -> Self {
        let mut names: Vec<String> = config
            .pack_set
            .packs
            .iter()
            .map(|pack| pack.name.clone())
            .collect::<Vec<String>>()
            .into();
        names.sort();
        PackageNames { names }
    }
    // TODO: build up hash of dir -> pack as we go
    pub fn find_pack_name(&self, file_path: &str) -> Option<String> {
        let mut pack_name = ".";
        let mut containing = false;
        for pn in self.names.iter() {
            if file_path.contains(pn) {
                if pn.len() > pack_name.len() {
                    pack_name = pn;
                    containing = true;
                }
            } else {
                if containing {
                    break;
                }
            }
        }
        Some(pack_name.to_string())
    }
}

impl ExtraReferenceFieldsFn for PackageNames {
    fn extra_reference_fields_fn(
        &self,
        relative_referencing_file: &str,
        relative_defining_file: Option<&str>,
    ) -> HashMap<String, String> {
        let mut extra_fields = HashMap::new();
        if let Some(referencing_pack) =
            self.find_pack_name(relative_referencing_file)
        {
            extra_fields
                .insert("referencing_pack_name".to_string(), referencing_pack);
        }
        if let Some(defining_file) = relative_defining_file {
            if let Some(defining_pack) = self.find_pack_name(defining_file) {
                extra_fields
                    .insert("defining_pack_name".to_string(), defining_pack);
            }
        }
        extra_fields
    }
}

fn autoload_paths_from_config(
    configuration: &Configuration,
) -> HashMap<PathBuf, String> {
    let mut autoload_paths: HashMap<PathBuf, String> = configuration
        .pack_set
        .packs
        .iter()
        .flat_map(|pack| pack.default_autoload_roots())
        .map(|path| (path, String::from("")))
        .collect();
    configuration
        .autoload_roots
        .iter()
        .for_each(|(rel_path, ns)| {
            let abs_path = configuration.absolute_root.join(rel_path);
            let ns = if ns == "::Object" {
                String::from("")
            } else {
                ns.to_owned()
            };
            expand_glob(abs_path.to_str().unwrap())
                .iter()
                .for_each(|path| {
                    autoload_paths.insert(path.to_owned(), ns.clone());
                });
        });
    autoload_paths
}

pub(crate) fn get_all_references_new(
    configuration: &Configuration,
    absolute_paths: &HashSet<PathBuf>,
) -> anyhow::Result<Vec<Reference>> {
    let pack_names = PackageNames::new(configuration);

    // todo: still need the special case roots
    let extra_reference_fields_fn =
        Some(Box::new(pack_names) as Box<dyn ExtraReferenceFieldsFn>);
    let ref_config = ruby_references::configuration::Configuration {
        absolute_root: configuration.absolute_root.clone(),
        autoload_paths: autoload_paths_from_config(configuration),
        acronyms: rails_utils::get_acronyms_from_disk(
            &configuration.inflections_path,
        ),
        included_files: absolute_paths.clone(),
        include_reference_is_definition: false,
        extra_reference_fields_fn,
        ..Default::default()
    };

    let refs = ruby_references::reference::all_references(&ref_config)?;
    let pks_references = refs
        .into_iter()
        .filter_map(|r| {
            if r.extra_fields.get("referencing_pack_name").is_none() {
                None
            } else {
                Some(Reference {
                    constant_name: r.constant_name,
                    defining_pack_name: r
                        .extra_fields
                        .get("defining_pack_name")
                        .cloned(),
                    relative_defining_file: r.relative_defining_file,
                    referencing_pack_name: r
                        .extra_fields
                        .get("referencing_pack_name")
                        .cloned()
                        .unwrap(),
                    relative_referencing_file: r.relative_referencing_file,
                    source_location: SourceLocation {
                        line: r.source_location.line,
                        column: r.source_location.column,
                    },
                })
            }
        })
        .collect();

    Ok(pks_references)
}
