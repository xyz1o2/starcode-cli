use std::collections::{HashMap, HashSet};

/// Code structure index - understands function calls, type dependencies, imports
#[derive(Clone)]
pub struct StructureIndex {
    /// Function definitions: name -> (file, line, signature, body_hash)
    pub functions: HashMap<String, Vec<FunctionDef>>,
    /// Type definitions: name -> (file, line, kind)
    pub types: HashMap<String, Vec<TypeDef>>,
    /// Import relationships: file -> [(imported_module, imported_name)]
    pub imports: HashMap<String, Vec<Import>>,
    /// Call graph: function -> [called_functions]
    pub call_graph: HashMap<String, HashSet<String>>,
    /// Reverse call graph: function -> [calling_functions]
    pub reverse_call_graph: HashMap<String, HashSet<String>>,
    /// Last index time
    pub last_indexed: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub file: String,
    pub line: usize,
    pub end_line: usize,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub visibility: String,  // pub, pub(crate), private
    pub is_async: bool,
    pub body_hash: String,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub file: String,
    pub line: usize,
    pub kind: TypeKind,
    pub fields: Vec<String>,
    pub methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum TypeKind {
    Struct,     // Rust struct, C struct, Go struct
    Enum,       // enum in any language
    Interface,  // trait (Rust), interface (Java/TS/Go), protocol (Swift), ABC (Python)
    Class,      // class (Python/JS/Java/C++/Ruby)
    TypeAlias,  // type alias, typedef
    Module,     // mod (Rust), package (Java), namespace (C++/TS)
}

#[derive(Debug, Clone)]
pub struct Import {
    pub module: String,
    pub name: String,
    pub alias: Option<String>,
}

impl StructureIndex {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            types: HashMap::new(),
            imports: HashMap::new(),
            call_graph: HashMap::new(),
            reverse_call_graph: HashMap::new(),
            last_indexed: None,
        }
    }

    /// Index a single file.
    /// NB: regex-based parsing is a placeholder — real parsing should use tree-sitter.
    pub fn index_file(&mut self, path: &str, content: &str) {
        let ext = path.rsplit('.').next().unwrap_or("");
        match ext {
            "rs" => self.index_rust(path, content),
            "py" => self.index_python(path, content),
            "js" | "jsx" => self.index_js_ts(path, content),
            "ts" | "tsx" => self.index_js_ts(path, content),
            _ => {}
        }
    }

    /// Find all references to a function
    pub fn find_references(&self, name: &str) -> Vec<Reference> {
        let mut refs = Vec::new();
        
        // Direct calls
        if let Some(callers) = self.reverse_call_graph.get(name) {
            for caller in callers {
                if let Some(funcs) = self.functions.get(caller) {
                    for func in funcs {
                        refs.push(Reference {
                            file: func.file.clone(),
                            line: func.line,
                            kind: RefKind::Call,
                        });
                    }
                }
            }
        }

        // Imports
        for (file, imports) in &self.imports {
            for import in imports {
                if import.name == name || import.alias.as_deref() == Some(name) {
                    refs.push(Reference {
                        file: file.clone(),
                        line: 0, // Will be resolved
                        kind: RefKind::Import,
                    });
                }
            }
        }

        refs
    }

    /// Find related functions (callers and callees)
    pub fn find_related(&self, name: &str) -> Vec<String> {
        let mut related = HashSet::new();
        
        // Callees
        if let Some(callees) = self.call_graph.get(name) {
            related.extend(callees.iter().cloned());
        }
        
        // Callers
        if let Some(callers) = self.reverse_call_graph.get(name) {
            related.extend(callers.iter().cloned());
        }

        related.into_iter().collect()
    }

    /// Find functions by partial name or signature
    pub fn search_functions(&self, query: &str) -> Vec<&FunctionDef> {
        let query_lower = query.to_lowercase();
        self.functions.values()
            .flatten()
            .filter(|f| {
                f.signature.to_lowercase().contains(&query_lower)
                || f.file.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    /// Get context for editing a function - returns related code
    pub fn get_edit_context(&self, name: &str) -> EditContext {
        let mut context = EditContext::default();
        
        // The function itself
        if let Some(funcs) = self.functions.get(name) {
            context.target_functions = funcs.clone();
        }
        
        // Related functions (callers and callees)
        let related = self.find_related(name);
        for rel in &related {
            if let Some(funcs) = self.functions.get(rel) {
                context.related_functions.extend(funcs.clone());
            }
        }
        
        // Types used by the function
        context.related_types = self.find_types_for_function(name);
        
        // Imports
        for func in &context.target_functions {
            if let Some(imports) = self.imports.get(&func.file) {
                context.imports = imports.clone();
            }
        }
        
        context
    }

    pub fn find_types_for_function(&self, name: &str) -> Vec<TypeDef> {
        // Simple heuristic: types in the same file as the function
        if let Some(funcs) = self.functions.get(name) {
            let files: HashSet<&str> = funcs.iter().map(|f| f.file.as_str()).collect();
            self.types.values()
                .flatten()
                .filter(|t| files.contains(t.file.as_str()))
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    }

    fn index_rust(&mut self, path: &str, content: &str) {
        // Parse Rust code using regex patterns
        let func_regex = regex::Regex::new(
            r"(?m)^(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(([^)]*)\)(?:\s*->\s*([^\{]+))?\s*\{"
        ).unwrap();
        
        let struct_regex = regex::Regex::new(
            r"(?m)^(?:pub\s+)?struct\s+(\w+)"
        ).unwrap();

        let enum_regex = regex::Regex::new(
            r"(?m)^(?:pub\s+)?enum\s+(\w+)"
        ).unwrap();

        let impl_regex = regex::Regex::new(
            r"(?m)^impl\s+(?:<[^>]*>\s+)?(\w+)"
        ).unwrap();

        let use_regex = regex::Regex::new(
            r"(?m)^use\s+([\w:]+)(?:\s+as\s+(\w+))?;"
        ).unwrap();

        // Index functions
        for cap in func_regex.captures_iter(content) {
            let name = cap[1].to_string();
            let params = cap[2].to_string();
            let return_type = cap.get(3).map(|m| m.as_str().trim().to_string());
            let line = content[..cap.get(0).unwrap().start()].lines().count();
            
            let signature = format!("fn {}({}){}", name, params, 
                return_type.map(|r| format!(" -> {}", r)).unwrap_or_default());
            
            let func_def = FunctionDef {
                file: path.to_string(),
                line,
                end_line: line + 10, // Approximate
                signature,
                doc_comment: None,
                visibility: if cap[0].contains("pub") { "pub".to_string() } else { "private".to_string() },
                is_async: cap[0].contains("async"),
                body_hash: format!("{:x}", md5::compute(&cap[0])),
            };
            
            self.functions.entry(name).or_insert_with(Vec::new).push(func_def);
        }

        // Index structs
        for cap in struct_regex.captures_iter(content) {
            let name = cap[1].to_string();
            let line = content[..cap.get(0).unwrap().start()].lines().count();
            
            let type_def = TypeDef {
                file: path.to_string(),
                line,
                kind: TypeKind::Struct,
                fields: Vec::new(),
                methods: Vec::new(),
            };
            
            self.types.entry(name).or_insert_with(Vec::new).push(type_def);
        }

        // Index enums
        for cap in enum_regex.captures_iter(content) {
            let name = cap[1].to_string();
            let line = content[..cap.get(0).unwrap().start()].lines().count();
            
            let type_def = TypeDef {
                file: path.to_string(),
                line,
                kind: TypeKind::Enum,
                fields: Vec::new(),
                methods: Vec::new(),
            };
            
            self.types.entry(name).or_insert_with(Vec::new).push(type_def);
        }

        // Index impls
        for cap in impl_regex.captures_iter(content) {
            let name = cap[1].to_string();
            let line = content[..cap.get(0).unwrap().start()].lines().count();
            
            let type_def = TypeDef {
                file: path.to_string(),
                line,
                kind: TypeKind::Interface, // Rust `impl` blocks → interface
                fields: Vec::new(),
                methods: Vec::new(),
            };
            
            self.types.entry(name).or_insert_with(Vec::new).push(type_def);
        }

        // Index imports
        for cap in use_regex.captures_iter(content) {
            let module = cap[1].to_string();
            let alias = cap.get(2).map(|m| m.as_str().to_string());
            let name = module.split("::").last().unwrap_or(&module).to_string();
            
            let import = Import {
                module,
                name,
                alias,
            };
            
            self.imports.entry(path.to_string()).or_insert_with(Vec::new).push(import);
        }

        // Build call graph
        self.build_call_graph(path, content);
    }

    fn index_python(&mut self, path: &str, content: &str) {
        // Similar implementation for Python
        let func_regex = regex::Regex::new(
            r"(?m)^(?:async\s+)?def\s+(\w+)\s*\(([^)]*)\)(?:\s*->\s*([^\:]+))?\s*:"
        ).unwrap();

        for cap in func_regex.captures_iter(content) {
            let name = cap[1].to_string();
            let params = cap[2].to_string();
            let return_type = cap.get(3).map(|m| m.as_str().trim().to_string());
            let line = content[..cap.get(0).unwrap().start()].lines().count();
            
            let signature = format!("def {}({}){}", name, params,
                return_type.map(|r| format!(" -> {}", r)).unwrap_or_default());
            
            let func_def = FunctionDef {
                file: path.to_string(),
                line,
                end_line: line + 10,
                signature,
                doc_comment: None,
                visibility: "public".to_string(),
                is_async: cap[0].contains("async"),
                body_hash: format!("{:x}", md5::compute(&cap[0])),
            };
            
            self.functions.entry(name).or_insert_with(Vec::new).push(func_def);
        }
    }

    fn index_js_ts(&mut self, path: &str, content: &str) {
        // Similar implementation for JavaScript/TypeScript
        let func_regex = regex::Regex::new(
            r"(?m)^(?:export\s+)?(?:async\s+)?function\s+(\w+)\s*\(([^)]*)\)(?:\s*:\s*([^\{]+))?\s*\{"
        ).unwrap();

        for cap in func_regex.captures_iter(content) {
            let name = cap[1].to_string();
            let params = cap[2].to_string();
            let return_type = cap.get(3).map(|m| m.as_str().trim().to_string());
            let line = content[..cap.get(0).unwrap().start()].lines().count();
            
            let signature = format!("function {}({}){}", name, params,
                return_type.map(|r| format!(": {}", r)).unwrap_or_default());
            
            let func_def = FunctionDef {
                file: path.to_string(),
                line,
                end_line: line + 10,
                signature,
                doc_comment: None,
                visibility: if cap[0].contains("export") { "public".to_string() } else { "private".to_string() },
                is_async: cap[0].contains("async"),
                body_hash: format!("{:x}", md5::compute(&cap[0])),
            };
            
            self.functions.entry(name).or_insert_with(Vec::new).push(func_def);
        }
    }

    fn build_call_graph(&mut self, path: &str, content: &str) {
        // Find all function calls in the file
        let call_regex = regex::Regex::new(r"(\w+)\s*\(").unwrap();
        
        // Get all functions defined in this file
        let file_funcs: Vec<String> = self.functions.values()
            .flatten()
            .filter(|f| f.file == path)
            .map(|f| f.signature.split('(').next().unwrap_or("").to_string())
            .collect();

        for func_name in &file_funcs {
            let mut callees = HashSet::new();
            
            // Find all calls in the function body
            // This is simplified - in production, use AST parsing
            for cap in call_regex.captures_iter(content) {
                let called = cap[1].to_string();
                if called != *func_name && self.functions.contains_key(&called) {
                    callees.insert(called.clone());
                    
                    // Update reverse call graph
                    self.reverse_call_graph
                        .entry(called)
                        .or_insert_with(HashSet::new)
                        .insert(func_name.clone());
                }
            }
            
            self.call_graph.insert(func_name.clone(), callees);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EditContext {
    pub target_functions: Vec<FunctionDef>,
    pub related_functions: Vec<FunctionDef>,
    pub related_types: Vec<TypeDef>,
    pub imports: Vec<Import>,
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub file: String,
    pub line: usize,
    pub kind: RefKind,
}

#[derive(Debug, Clone)]
pub enum RefKind {
    Call,
    Import,
    Definition,
}

