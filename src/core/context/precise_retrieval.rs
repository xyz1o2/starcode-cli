use super::structure_index::{Import, StructureIndex};

/// Precise retrieval - get only the relevant code for a task
pub struct PreciseRetriever {
    index: StructureIndex,
}

impl PreciseRetriever {
    pub fn new(index: StructureIndex) -> Self {
        Self { index }
    }

    /// Retrieve context for editing a specific function
    pub fn for_edit(&self, function_name: &str, file_path: &str) -> RetrievalResult {
        let mut result = RetrievalResult::default();

        // 1. Get the function definition
        if let Some(funcs) = self.index.functions.get(function_name) {
            for func in funcs {
                result.primary_code.push(CodeSlice {
                    file: func.file.clone(),
                    start_line: func.line,
                    end_line: func.end_line,
                    content: func.signature.clone(),
                    relevance: 1.0,
                });
            }
        }

        // 2. Get related functions (callers and callees)
        let related = self.index.find_related(function_name);
        for rel in &related {
            if let Some(funcs) = self.index.functions.get(rel) {
                for func in funcs {
                    result.related_code.push(CodeSlice {
                        file: func.file.clone(),
                        start_line: func.line,
                        end_line: func.end_line,
                        content: func.signature.clone(),
                        relevance: 0.8,
                    });
                }
            }
        }

        // 3. Get types used by the function
        let types = self.index.find_types_for_function(function_name);
        for t in &types {
            result.type_definitions.push(TypeSlice {
                file: t.file.clone(),
                line: t.line,
                name: format!("{:?}", t.kind),
                relevance: 0.7,
            });
        }

        // 4. Get imports
        if let Some(funcs) = self.index.functions.get(function_name) {
            for func in funcs {
                if let Some(imports) = self.index.imports.get(&func.file) {
                    result.imports = imports.clone();
                }
            }
        }

        result
    }

    /// Retrieve context for understanding a file
    pub fn for_file(&self, file_path: &str) -> RetrievalResult {
        let mut result = RetrievalResult::default();

        // Get all functions in the file
        for funcs in self.index.functions.values() {
            for func in funcs {
                if func.file == file_path {
                    result.primary_code.push(CodeSlice {
                        file: func.file.clone(),
                        start_line: func.line,
                        end_line: func.end_line,
                        content: func.signature.clone(),
                        relevance: 1.0,
                    });
                }
            }
        }

        // Get all types in the file
        for types in self.index.types.values() {
            for t in types {
                if t.file == file_path {
                    result.type_definitions.push(TypeSlice {
                        file: t.file.clone(),
                        line: t.line,
                        name: format!("{:?}", t.kind),
                        relevance: 1.0,
                    });
                }
            }
        }

        result
    }

    /// Retrieve context for a search query
    pub fn for_query(&self, query: &str) -> RetrievalResult {
        let mut result = RetrievalResult::default();

        let functions = self.index.search_functions(query);
        for func in functions {
            result.primary_code.push(CodeSlice {
                file: func.file.clone(),
                start_line: func.line,
                end_line: func.end_line,
                content: func.signature.clone(),
                relevance: 0.9,
            });
        }

        result
    }

    /// Get the underlying index
    pub fn index(&self) -> &StructureIndex {
        &self.index
    }

    /// Get mutable reference to the underlying index
    pub fn index_mut(&mut self) -> &mut StructureIndex {
        &mut self.index
    }
}

#[derive(Debug, Default)]
pub struct RetrievalResult {
    pub primary_code: Vec<CodeSlice>,
    pub related_code: Vec<CodeSlice>,
    pub type_definitions: Vec<TypeSlice>,
    pub imports: Vec<Import>,
}

#[derive(Debug)]
pub struct CodeSlice {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub relevance: f64,
}

#[derive(Debug)]
pub struct TypeSlice {
    pub file: String,
    pub line: usize,
    pub name: String,
    pub relevance: f64,
}
