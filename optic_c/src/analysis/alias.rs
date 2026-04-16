pub struct AliasAnalysis {
    vulnerabilities: Vec<Vulnerability>,
}

#[derive(Debug, Clone)]
pub struct Vulnerability {
    pub line: u32,
    pub kind: VulnerabilityKind,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum VulnerabilityKind {
    BufferOverflow,
    UseAfterFree,
    DoubleFree,
    NullDereference,
    IntegerOverflow,
    CommandInjection,
}

impl AliasAnalysis {
    pub fn new() -> Self {
        Self {
            vulnerabilities: Vec::new(),
        }
    }

    pub fn add_vulnerability(&mut self, line: u32, kind: VulnerabilityKind, message: &str) {
        self.vulnerabilities.push(Vulnerability {
            line,
            kind,
            message: message.to_string(),
        });
    }

    pub fn is_vulnerable(&self, line: &str) -> bool {
        let dangerous_patterns = [
            "strcpy",
            "sprintf",
            "gets",
            "scanf",
            "strcat",
            "malloc",
            "calloc",
        ];
        
        for pattern in &dangerous_patterns {
            if line.contains(pattern) {
                return true;
            }
        }
        
        false
    }

    pub fn has_vulnerabilities(&self) -> bool {
        !self.vulnerabilities.is_empty()
    }

    pub fn get_vulnerabilities_at_line(&self, line: u32) -> Vec<&Vulnerability> {
        self.vulnerabilities
            .iter()
            .filter(|v| v.line == line)
            .collect()
    }

    pub fn query_node_vulnerabilities(&self, node_offset: u32) -> Vec<&Vulnerability> {
        self.vulnerabilities
            .iter()
            .filter(|v| v.line == node_offset)
            .collect()
    }
}

impl Default for AliasAnalysis {
    fn default() -> Self {
        Self::new()
    }
}