/// A versatile chunker inspired by Chonkie's RecursiveChunker.
/// It recursively splits text using a list of separators to ensure chunks fit within a target size
/// while preserving semantic boundaries as much as possible.
pub struct RecursiveChunker {
    pub chunk_size: usize,
    pub chunk_overlap: usize,
    pub separators: Vec<String>,
}

impl Default for RecursiveChunker {
    fn default() -> Self {
        Self {
            chunk_size: 512, // Default char size target (approx 100-150 tokens)
            chunk_overlap: 50,
            separators: vec![
                "\n\n".to_string(),
                "\n".to_string(),
                ". ".to_string(), // Sentence boundary
                "? ".to_string(),
                "! ".to_string(),
                " ".to_string(),
                "".to_string(), // Char level fallback
            ],
        }
    }
}

impl RecursiveChunker {
    pub fn new(chunk_size: usize, chunk_overlap: usize) -> Self {
        Self {
            chunk_size,
            chunk_overlap,
            ..Default::default()
        }
    }

    pub fn with_separators(mut self, separators: Vec<String>) -> Self {
        self.separators = separators;
        self
    }

    /// Main entry point to chunk text
    pub fn chunk(&self, text: &str) -> Vec<CodeChunk> {
        let raw_chunks = self.split_text(text, &self.separators);

        // Convert raw string chunks to CodeChunks with line numbers
        let mut chunks = Vec::new();
        let mut current_line = 1;

        for content in raw_chunks {
            let line_count = content.lines().count();
            let start_line = current_line;
            let end_line = start_line + line_count.saturating_sub(1);

            // Refine line counting: find actual start line in original text if possible?
            // Since we are splitting recursively, exact line mapping is tricky without tracking indices.
            // For now, we estimate or accumulate. A better approach for exact lines
            // is to match the chunk content back to the original text, but that's expensive.
            // Simplified: we just increment, assuming sequential chunks.
            // Note: This assumes chunks are sequential and cover the whole text (mostly true).

            chunks.push(CodeChunk {
                content: content.clone(),
                start_line,
                end_line,
                context_header: None, // Recursive chunker is generic, context header logic needs AST
            });

            // Update current line for next chunk.
            // Note: Overlap makes this tricky. The next chunk might start earlier.
            // But for simple visualization, this is okay.
            // Ideally we pass original text and find offsets.
            current_line += line_count;
        }

        chunks
    }

    fn split_text(&self, text: &str, separators: &[String]) -> Vec<String> {
        let mut final_chunks = Vec::new();

        // 1. Find the best separator that works
        let mut separator = separators.last().unwrap().as_str();
        let mut next_separators = &separators[..];

        for (i, sep) in separators.iter().enumerate() {
            if sep.is_empty() {
                separator = "";
                next_separators = &separators[..0]; // No more separators
                break;
            }
            if text.contains(sep) {
                separator = sep;
                next_separators = &separators[i + 1..];
                break;
            }
        }

        // 2. Split using the separator
        let splits: Vec<String> = if separator.is_empty() {
            text.chars().map(|c| c.to_string()).collect()
        } else {
            text.split(separator).map(|s| s.to_string()).collect()
        };

        // 3. Merge splits into chunks
        let mut current_chunk = String::new();

        for split in splits {
            let sep_len = if separator.is_empty() {
                0
            } else {
                separator.len()
            };
            let split_len = split.len();

            // If adding this split exceeds chunk size
            if current_chunk.len() + split_len + sep_len > self.chunk_size {
                if !current_chunk.is_empty() {
                    // Current chunk is ready
                    final_chunks.push(current_chunk.clone());
                    current_chunk.clear();
                }

                // If the single split itself is too big, recurse on it
                if split_len > self.chunk_size && !next_separators.is_empty() {
                    let sub_chunks = self.split_text(&split, next_separators);
                    final_chunks.extend(sub_chunks);
                } else {
                    // Start new chunk with this split
                    current_chunk.push_str(&split);
                }
            } else {
                // Append to current chunk
                if !current_chunk.is_empty() {
                    current_chunk.push_str(separator);
                }
                current_chunk.push_str(&split);
            }
        }

        if !current_chunk.is_empty() {
            final_chunks.push(current_chunk);
        }

        final_chunks
    }
}

/// A simple structure-aware chunker that splits code based on indentation and common block markers.
/// This is a heuristic-based approach, lighter than full AST parsing but better than line-based splitting.
pub struct SmartChunker;

#[derive(Debug, Clone, PartialEq)]
pub struct CodeChunk {
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub context_header: Option<String>, // E.g., "fn process_data() {"
}

impl SmartChunker {
    /// Splits source code into meaningful chunks with a 3-layer fallback:
    ///
    ///   Layer 1: Tree-sitter AST-aware chunking (best quality, P1)
    ///   Layer 2: Heuristic (curly-brace / indentation)
    ///   Layer 3: Text-based sliding window (guaranteed non-empty for non-empty input)
    ///
    /// Every layer produces empty results → the next layer tries. Layer 3
    /// guarantees at least one chunk for any non-empty input, so the indexing
    /// pipeline never silently drops a file.
    pub fn chunk(content: &str, file_ext: &str) -> Vec<CodeChunk> {
        // ── Layer 1: Tree-sitter AST-aware chunking ────────────────────────────
        match file_ext {
            "rs" | "py" | "pyi" | "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "go"
            | "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "java" => {
                let ts_chunks = super::tree_sitter_chunker::chunk_with_tree_sitter(content, file_ext);
                if !ts_chunks.is_empty() {
                    return ts_chunks;
                }
            }
            _ => {}
        }

        // ── Layer 2: Heuristic fallback ───────────────────────────────────────
        let heuristic = match file_ext {
            "rs" | "c" | "cpp" | "java" | "js" | "ts" | "go" => Self::chunk_curly_braces(content),
            "py" => Self::chunk_indentation(content),
            _ => Vec::new(),
        };
        if !heuristic.is_empty() {
            return heuristic;
        }

        // ── Layer 3: Text-based RecursiveChunker (guaranteed fallback) ────────
        // This is the safety net.  RecursiveChunker splits by paragraph / sentence /
        // word / character boundaries and always produces at least one chunk for
        // non-empty input.
        if content.trim().is_empty() {
            return Vec::new();
        }

        let chunker = match file_ext {
            "md" | "txt" => RecursiveChunker::new(1000, 100).with_separators(vec![
                "\n\n".to_string(),
                "\n# ".to_string(),
                "\n".to_string(),
                ". ".to_string(),
                " ".to_string(),
                "".to_string(),
            ]),
            _ => RecursiveChunker::new(512, 50),
        };
        let text_chunks = chunker.chunk(content);
        if !text_chunks.is_empty() {
            return text_chunks;
        }

        // Absolute last resort: single chunk for the whole file
        let lines: Vec<&str> = content.lines().collect();
        vec![CodeChunk {
            content: content.to_string(),
            start_line: 1,
            end_line: lines.len().max(1),
            context_header: None,
        }]
    }

    /// For C-like languages (Rust, JS, Java, etc.)
    fn chunk_curly_braces(content: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        let mut chunks = Vec::new();
        let mut current_chunk_lines = Vec::new();
        let mut chunk_start_idx = 0;
        let mut brace_balance = 0;

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            // Skip comments (simplified)
            if trimmed.starts_with("//") {
                current_chunk_lines.push(*line);
                continue;
            }

            // Update brace balance
            brace_balance += trimmed.matches('{').count() as i32;
            brace_balance -= trimmed.matches('}').count() as i32;

            current_chunk_lines.push(*line);

            // If balance returns to 0 (or less) and we have enough lines, emit chunk
            // Or if chunk gets too big (>50 lines)
            if (brace_balance <= 0 && current_chunk_lines.len() >= 5)
                || current_chunk_lines.len() > 50
            {
                // If it's a small chunk (<3 lines), keep accumulating unless it's a clear block end
                if current_chunk_lines.len() < 3 && brace_balance > 0 {
                    continue;
                }

                chunks.push(CodeChunk {
                    content: current_chunk_lines.join("\n"),
                    start_line: chunk_start_idx + 1,
                    end_line: i + 1,
                    context_header: lines.get(chunk_start_idx).map(|s| s.trim().to_string()),
                });

                current_chunk_lines.clear();
                chunk_start_idx = i + 1;
                brace_balance = 0; // Reset for robustness
            }
        }

        // Remaining lines
        if !current_chunk_lines.is_empty() {
            chunks.push(CodeChunk {
                content: current_chunk_lines.join("\n"),
                start_line: chunk_start_idx + 1,
                end_line: lines.len(),
                context_header: None,
            });
        }

        chunks
    }

    /// For Python-like languages (indentation based)
    fn chunk_indentation(content: &str) -> Vec<CodeChunk> {
        // Simplified: Split on top-level definitions (def, class)
        let lines: Vec<&str> = content.lines().collect();
        let mut chunks = Vec::new();
        let mut current_chunk_lines = Vec::new();
        let mut chunk_start_idx = 0;

        for (i, line) in lines.iter().enumerate() {
            // Check for top-level definition (no indentation)
            if (line.starts_with("def ") || line.starts_with("class ") || line.starts_with("@"))
                && !line.starts_with(" ")
            {
                if !current_chunk_lines.is_empty() {
                    let content = current_chunk_lines.join("\n");
                    if !content.trim().is_empty() {
                        chunks.push(CodeChunk {
                            content,
                            start_line: chunk_start_idx + 1,
                            end_line: i,
                            context_header: lines
                                .get(chunk_start_idx)
                                .map(|s| s.trim().to_string()),
                        });
                    }
                    current_chunk_lines.clear();
                    chunk_start_idx = i;
                }
            }
            current_chunk_lines.push(*line);
        }

        if !current_chunk_lines.is_empty() {
            let content = current_chunk_lines.join("\n");
            if !content.trim().is_empty() {
                chunks.push(CodeChunk {
                    content,
                    start_line: chunk_start_idx + 1,
                    end_line: lines.len(),
                    context_header: None,
                });
            }
        }
        chunks
    }
}
 