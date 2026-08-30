use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextDefinition {
    // Basic info
    pub id: String,          // Unique identifier
    pub name: String,        // Display name
    pub description: String, // Description
    pub content: String,     // Actual content

    // Metadata
    pub metadata: ContextMetadata,          // Detailed metadata
    pub tags: HashMap<String, Vec<String>>, // Category tags

    // Dependencies and priority
    pub dependencies: Vec<String>, // IDs of other contexts this depends on
    pub priority: i32,             // Priority (higher number = higher priority)
    pub version: String,           // Version number
}

impl ContextDefinition {
    pub fn new(id: String, name: String, content: String) -> Self {
        Self {
            id,
            name,
            description: String::new(),
            content,
            metadata: ContextMetadata::default(),
            tags: HashMap::new(),
            dependencies: Vec::new(),
            priority: 0,
            version: "1.0.0".to_string(),
        }
    }

    // Tag operations
    pub fn add_tag(&mut self, category: String, tag: String) {
        self.tags.entry(category).or_insert_with(Vec::new).push(tag);
    }

    pub fn has_tag(&self, category: &str, tag: &str) -> bool {
        self.tags
            .get(category)
            .map(|tags| tags.contains(&tag.to_string()))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMetadata {
    // Time information
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // Author info
    pub author: Option<String>,
    pub maintainers: Vec<String>,

    // Project adaptation info
    pub project_types: Vec<String>,      // Applicable project types
    pub tech_stack: Vec<String>,         // Tech stack
    pub file_patterns: Vec<String>,      // File patterns (glob)
    pub directory_patterns: Vec<String>, // Directory patterns

    // Matching weight
    pub weight: f64, // Base weight (0.0-1.0)

    // Source
    pub source: ContextSource,

    // Usage stats
    pub usage_count: u64,
    pub last_used_at: Option<DateTime<Utc>>,

    // Version control
    pub version: String,
    pub changelog: Vec<String>,
}

impl Default for ContextMetadata {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            created_at: now,
            updated_at: now,
            author: None,
            maintainers: Vec::new(),
            project_types: Vec::new(),
            tech_stack: Vec::new(),
            file_patterns: Vec::new(),
            directory_patterns: Vec::new(),
            weight: 1.0,
            source: ContextSource::ProjectFile("".to_string()),
            usage_count: 0,
            last_used_at: None,
            version: "1.0.0".to_string(),
            changelog: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextSource {
    ProjectFile(String),     // Project file: CLAUDE.md, STAR.md
    WorkspaceConfig(String), // Workspace config: .star/contexts/
    UserGlobal(String),      // User global: ~/.star/contexts/
    RemoteURL(String),       // Remote URL
    Extension(String),       // Extension provided
    System(String),          // Built-in system
}

impl ContextSource {
    pub fn get_type(&self) -> &str {
        match self {
            ContextSource::ProjectFile(_) => "project",
            ContextSource::WorkspaceConfig(_) => "workspace",
            ContextSource::UserGlobal(_) => "user",
            ContextSource::RemoteURL(_) => "remote",
            ContextSource::Extension(_) => "extension",
            ContextSource::System(_) => "system",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextLayer {
    // Layer info
    pub level: ContextLevel,
    pub definition: ContextDefinition,

    // State info
    pub active: bool,
    pub loaded_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,

    // Runtime data
    pub runtime_data: HashMap<String, String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextLevel {
    System = 0,     // System level (lowest priority)
    UserGlobal = 1, // User global
    Workspace = 2,  // Workspace
    Project = 3,    // Project level
    Directory = 4,  // Directory level
    File = 5,       // File level
    Session = 6,    // Session level (highest priority)
}

impl ContextLayer {
    pub fn new(level: ContextLevel, definition: ContextDefinition) -> Self {
        Self {
            level,
            definition,
            active: false,
            loaded_at: None,
            expires_at: None,
            runtime_data: HashMap::new(),
            errors: Vec::new(),
        }
    }

    pub fn activate(&mut self) {
        self.active = true;
        self.loaded_at = Some(Utc::now());
    }

    pub fn deactivate(&mut self) {
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_expired(&self) -> bool {
        if let Some(expires) = self.expires_at {
            Utc::now() > expires
        } else {
            false
        }
    }

    pub fn set_expiration(&mut self, duration: Duration) {
        self.expires_at = Some(Utc::now() + chrono::Duration::from_std(duration).unwrap());
    }
}

#[derive(Debug, Clone)]
pub struct ContextMatchScore {
    pub context_id: String,
    pub context_name: String,
    pub score: f64,                 // Total score (0.0-1.0)
    pub matches: Vec<ContextMatch>, // Detailed match info
    pub project_features: ProjectFeatures,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ContextMatch {
    pub metric: MatchMetric,
    pub score: f64,      // Item score
    pub details: String, // Detailed explanation
    pub weight: f64,     // Weight
}

#[derive(Debug, Clone)]
pub enum MatchMetric {
    TechStackOverlap(f64),        // Tech stack overlap
    FilePatternMatch(f64),        // File pattern match
    DirectoryStructureMatch(f64), // Directory structure match
    ProjectTypeMatch(f64),        // Project type match
    GitHistoryMatch(f64),         // Git history match
}

impl MatchMetric {
    pub fn get_name(&self) -> &str {
        match self {
            MatchMetric::TechStackOverlap(_) => "Tech Stack",
            MatchMetric::FilePatternMatch(_) => "File Patterns",
            MatchMetric::DirectoryStructureMatch(_) => "Directory Structure",
            MatchMetric::ProjectTypeMatch(_) => "Project Type",
            MatchMetric::GitHistoryMatch(_) => "Git History",
        }
    }

    pub fn get_score(&self) -> f64 {
        match self {
            MatchMetric::TechStackOverlap(s)
            | MatchMetric::FilePatternMatch(s)
            | MatchMetric::DirectoryStructureMatch(s)
            | MatchMetric::ProjectTypeMatch(s)
            | MatchMetric::GitHistoryMatch(s) => *s,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectFeatures {
    pub project_type: String,
    pub tech_stack: Vec<String>,
    pub file_patterns: Vec<String>,
    pub directory_structure: Vec<String>,
    pub git_history: Option<GitFeatures>,
}

#[derive(Debug, Clone)]
pub struct GitFeatures {
    pub main_branch: String,
    pub recent_commits: Vec<String>,
    pub frequent_files: Vec<String>,
    pub languages: Vec<String>,
}
