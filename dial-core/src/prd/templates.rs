/// A template section scaffold.
pub struct TemplateSection {
    pub section_id: &'static str,
    pub title: &'static str,
    pub level: i32,
    pub parent_id: Option<&'static str>,
    pub prompt_hint: &'static str,
}

/// A PRD template defining initial section structure.
pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub sections: &'static [TemplateSection],
}

/// Get a template by name.
pub fn get_template(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// List all available templates.
pub fn list_templates() -> &'static [Template] {
    TEMPLATES
}

static TEMPLATES: &[Template] = &[
    Template {
        name: "spec",
        description: "General specification with requirements and acceptance criteria",
        sections: &[
            TemplateSection { section_id: "1", title: "Problem Statement", level: 1, parent_id: None, prompt_hint: "What problem does this solve? Why does it need to be solved?" },
            TemplateSection { section_id: "2", title: "Requirements", level: 1, parent_id: None, prompt_hint: "What are the functional and non-functional requirements?" },
            TemplateSection { section_id: "2.1", title: "Functional Requirements", level: 2, parent_id: Some("2"), prompt_hint: "What must the system do? List specific behaviors." },
            TemplateSection { section_id: "2.2", title: "Non-Functional Requirements", level: 2, parent_id: Some("2"), prompt_hint: "Performance, reliability, scalability, security constraints?" },
            TemplateSection { section_id: "3", title: "Features", level: 1, parent_id: None, prompt_hint: "What are the user-facing features? Prioritize as must-have vs nice-to-have." },
            TemplateSection { section_id: "4", title: "Data Model", level: 1, parent_id: None, prompt_hint: "What are the core entities, their attributes, and relationships?" },
            TemplateSection { section_id: "5", title: "Constraints", level: 1, parent_id: None, prompt_hint: "Technical constraints, budget, timeline, platform requirements?" },
            TemplateSection { section_id: "6", title: "Acceptance Criteria", level: 1, parent_id: None, prompt_hint: "How do we know when each feature is done? What defines success?" },
        ],
    },
    Template {
        name: "architecture",
        description: "System architecture with components and deployment",
        sections: &[
            TemplateSection { section_id: "1", title: "Overview", level: 1, parent_id: None, prompt_hint: "High-level description of the system and its purpose." },
            TemplateSection { section_id: "2", title: "Components", level: 1, parent_id: None, prompt_hint: "What are the major components/modules and their responsibilities?" },
            TemplateSection { section_id: "2.1", title: "Component Interactions", level: 2, parent_id: Some("2"), prompt_hint: "How do the components communicate? What protocols or patterns?" },
            TemplateSection { section_id: "3", title: "Data Model", level: 1, parent_id: None, prompt_hint: "Database schema, data flow, storage strategy." },
            TemplateSection { section_id: "4", title: "Integrations", level: 1, parent_id: None, prompt_hint: "External services, APIs, third-party dependencies." },
            TemplateSection { section_id: "5", title: "Deployment", level: 1, parent_id: None, prompt_hint: "How is the system deployed? Infrastructure, CI/CD, environments." },
            TemplateSection { section_id: "6", title: "Security", level: 1, parent_id: None, prompt_hint: "Authentication, authorization, data protection, threat model." },
        ],
    },
    Template {
        name: "api",
        description: "API specification with endpoints and data types",
        sections: &[
            TemplateSection { section_id: "1", title: "Overview", level: 1, parent_id: None, prompt_hint: "What does this API do? Who consumes it?" },
            TemplateSection { section_id: "2", title: "Authentication", level: 1, parent_id: None, prompt_hint: "How do clients authenticate? API keys, OAuth, JWT?" },
            TemplateSection { section_id: "3", title: "Endpoints", level: 1, parent_id: None, prompt_hint: "List all endpoints with methods, paths, request/response bodies." },
            TemplateSection { section_id: "3.1", title: "Resource Endpoints", level: 2, parent_id: Some("3"), prompt_hint: "CRUD operations on primary resources." },
            TemplateSection { section_id: "3.2", title: "Action Endpoints", level: 2, parent_id: Some("3"), prompt_hint: "Non-CRUD operations, workflows, batch operations." },
            TemplateSection { section_id: "4", title: "Data Types", level: 1, parent_id: None, prompt_hint: "Request/response schemas, shared types, enums." },
            TemplateSection { section_id: "5", title: "Error Handling", level: 1, parent_id: None, prompt_hint: "Error codes, error response format, retry behavior." },
        ],
    },
    Template {
        name: "mvp",
        description: "Minimal viable product — lean and focused",
        sections: &[
            TemplateSection { section_id: "1", title: "Problem", level: 1, parent_id: None, prompt_hint: "What problem does this solve? Keep it to one paragraph." },
            TemplateSection { section_id: "2", title: "MVP Features", level: 1, parent_id: None, prompt_hint: "What is the absolute minimum set of features to be useful?" },
            TemplateSection { section_id: "3", title: "Technical Stack", level: 1, parent_id: None, prompt_hint: "Language, framework, database, hosting. Keep it simple." },
            TemplateSection { section_id: "4", title: "Data Model", level: 1, parent_id: None, prompt_hint: "Core entities and their relationships. Minimal schema." },
        ],
    },
];
