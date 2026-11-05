//! Julia imports, nominal types and Base method dispatch.

use super::*;

impl<'a> Index<'a> {
    /// `include("x.jl")` makes the file's names visible; `using`/`import` of a module of the
    /// root makes its names visible and binds its name; other modules are external.
    pub(super) fn julia_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            if import.spec.ends_with(".jl") {
                if let Some(target) = self
                    .file_by_path(file, &import.spec)
                    .filter(|&target| target != file)
                {
                    imports.files.push((file, target));
                    let module = self.module_of(target);
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                continue;
            }
            let path = import.spec.trim_start_matches('.');
            let last = path.rsplit('.').next().unwrap_or(path);
            let modules = self.module_symbols(last, FileType::Julia);
            if modules.is_empty() {
                imports
                    .externals
                    .push(path.split('.').next().unwrap_or(path).to_owned());
                continue;
            }
            for &module in &modules {
                let target = self.symbols[module as usize].file;
                if target != file && !imports.files.contains(&(file, target)) {
                    imports.files.push((file, target));
                }
                let target_module = self.module_of(target);
                imports.globs.push(target_module);
                imports.modules.push(target_module);
            }
            imports
                .names
                .insert(last.to_owned(), Target::Symbols(modules));
        }
    }

    /// Resolves an unqualified Julia builtin against typed, source-backed Base extensions.
    pub(super) fn resolve_julia_base_call(
        &self,
        file: u32,
        imports: &Imports,
        facts: &Facts,
        reference_index: u32,
        reference: &Ref,
    ) -> Outcome {
        if reference.arity == facts::NO_ARITY {
            return Outcome::External;
        }
        let mut arguments = Vec::new();
        for argument in facts.call_arguments_for(reference_index) {
            let Some(binding) = self.binding(
                file,
                reference.scope,
                &argument.binding,
                reference.line,
                reference.byte,
            ) else {
                continue;
            };
            if !binding.declared {
                continue;
            }
            let Some(type_symbol) =
                self.julia_nominal_type(file, imports, reference.scope, &binding.type_name)
            else {
                continue;
            };
            if !matches!(
                self.symbols[type_symbol as usize].kind,
                SymKind::Struct | SymKind::Enum
            ) {
                continue;
            }
            arguments.push((argument.position, type_symbol));
        }
        if arguments.is_empty() {
            return Outcome::External;
        }

        let marker = format!("{JULIA_BASE_MODULE}.{}(", reference.name);
        let mut targets = Vec::new();
        for &candidate in self
            .by_name
            .get(reference.name.as_str())
            .into_iter()
            .flatten()
        {
            let symbol = &self.symbols[candidate as usize];
            if !symbol.kind.is_callable()
                || self.file_type(symbol.file) != Some(FileType::Julia)
                || !symbol.signature.contains(&marker)
                || symbol.signature.contains(" where ")
                || symbol.signature.trim_end().ends_with('…')
            {
                continue;
            }
            let Some(accepts_arity) = julia_method_accepts(
                &symbol.signature,
                &reference.name,
                usize::from(reference.arity),
            ) else {
                continue;
            };
            if !accepts_arity {
                continue;
            }
            let target_file = symbol.file;
            let target_scope = Some(candidate - self.base[target_file as usize]);
            let target_imports = &self.file_imports[target_file as usize];
            let mut matched_type = false;
            let mut incompatible = false;
            let mut formal_types = Vec::with_capacity(arguments.len());
            for &(position, actual_type) in &arguments {
                let position = usize::from(position);
                let Some(formal_name) =
                    julia_parameter_name(&symbol.signature, &reference.name, position)
                else {
                    formal_types.push(None);
                    continue;
                };
                let formal_type_name =
                    julia_parameter_type(&symbol.signature, &reference.name, position).or_else(
                        || {
                            self.facts(target_file)?
                                .bindings
                                .iter()
                                .filter(|binding| {
                                    binding.scope == target_scope
                                        && binding.name == formal_name
                                        && binding.declared
                                })
                                .min_by_key(|binding| binding.line)
                                .map(|binding| binding.type_name.as_str())
                        },
                    );
                let Some(formal_type_name) = formal_type_name else {
                    formal_types.push(None);
                    continue;
                };
                let Some(formal_type) = self.julia_nominal_type(
                    target_file,
                    target_imports,
                    target_scope,
                    formal_type_name,
                ) else {
                    formal_types.push(None);
                    continue;
                };
                if !matches!(
                    self.symbols[formal_type as usize].kind,
                    SymKind::Struct | SymKind::Enum | SymKind::Iface
                ) {
                    formal_types.push(None);
                    continue;
                }
                formal_types.push(Some(formal_type));
                if self.julia_subtype_of(actual_type, formal_type) {
                    matched_type = true;
                } else {
                    incompatible = true;
                    break;
                }
            }
            if matched_type && !incompatible {
                targets.push((candidate, formal_types));
            }
        }
        let complete_dispatch = arguments.len() == usize::from(reference.arity)
            && arguments
                .iter()
                .enumerate()
                .all(|(position, (actual, _))| usize::from(*actual) == position)
            && targets.iter().all(|(_, formal_types)| {
                formal_types.len() == arguments.len()
                    && formal_types.iter().all(|type_symbol| type_symbol.is_some())
            });
        let targets: Vec<_> = if complete_dispatch {
            targets
                .iter()
                .enumerate()
                .filter(|(index, (_, formal_types))| {
                    !targets
                        .iter()
                        .enumerate()
                        .any(|(other_index, (_, other_types))| {
                            other_index != *index
                                && self.julia_method_more_specific(other_types, formal_types)
                        })
                })
                .map(|(_, (candidate, _))| (*candidate, Confidence::Inferred))
                .collect()
        } else {
            targets
                .into_iter()
                .map(|(candidate, _)| (candidate, Confidence::Inferred))
                .collect()
        };
        if targets.is_empty() {
            Outcome::External
        } else {
            Outcome::Targets(targets, Confidence::Inferred)
        }
    }

    pub(super) fn julia_nominal_type(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        name: &str,
    ) -> Option<u32> {
        if !is_julia_nominal_type(name) {
            return None;
        }
        let (types, confidence) = if let Some((qualifier, name)) = split_qualified(name) {
            let reference = Ref {
                kind: RefKind::Type,
                name: name.to_owned(),
                qualifier: Some(qualifier.to_owned()),
                scope,
                line: 0,
                arity: facts::NO_ARITY,
                byte: crate::domain::facts::NO_BYTE,
            };
            let Outcome::Targets(targets, confidence) = self.resolve_ref(file, imports, &reference)
            else {
                return None;
            };
            (
                targets.into_iter().map(|(symbol, _)| symbol).collect(),
                confidence,
            )
        } else {
            self.resolve_type(file, Some(imports), scope, name)?
        };
        if confidence != Confidence::Exact || types.len() != 1 {
            return None;
        }
        let symbol = types[0];
        (self.file_type(self.symbols[symbol as usize].file) == Some(FileType::Julia)
            && self.symbols[symbol as usize].kind.is_type())
        .then_some(symbol)
    }

    pub(super) fn julia_subtype_of(&self, actual: u32, expected: u32) -> bool {
        let mut pending = vec![actual];
        let mut visited = HashSet::default();
        while let Some(current) = pending.pop() {
            if current == expected {
                return true;
            }
            if visited.insert(current) {
                pending.extend(self.supertypes.get(&current).into_iter().flatten().copied());
            }
        }
        false
    }

    pub(super) fn julia_method_more_specific(
        &self,
        candidate: &[Option<u32>],
        other: &[Option<u32>],
    ) -> bool {
        if candidate.len() != other.len() {
            return false;
        }
        let mut stricter = false;
        for (&candidate_type, &other_type) in candidate.iter().zip(other) {
            let (Some(candidate_type), Some(other_type)) = (candidate_type, other_type) else {
                return false;
            };
            if candidate_type != other_type {
                if !self.julia_subtype_of(candidate_type, other_type) {
                    return false;
                }
                stricter = true;
            }
        }
        stricter
    }
}
