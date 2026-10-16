//! Module dependency scanner and topological DAG scheduler for F*, Pulse, and Vale.

use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GraphError {
    #[error("Cyclic dependency detected: cycle contains module '{0}'")]
    CycleDetected(String),
    #[error("I/O error during AST scan: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct ModuleNode {
    pub name: String,
    pub path: PathBuf,
    pub is_interface: bool,
    pub opens: Vec<String>,
}

pub struct DependencyGraph {
    pub graph: DiGraph<ModuleNode, ()>,
    pub name_to_node: HashMap<String, NodeIndex>,
}

impl DependencyGraph {
    /// Recursively scans include directories for .fst, .fsti, and .vaf files, building a dependency DAG.
    pub fn build(include_dirs: &[PathBuf]) -> Result<Self, GraphError> {
        let mut modules = Vec::new();

        for dir in include_dirs {
            if !dir.exists() {
                continue;
            }
            for entry in walkdir::WalkDir::new(dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let path = entry.path().to_path_buf();
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                if matches!(ext, "fst" | "fsti" | "vaf")
                    && let Some(node) = Self::parse_module_header(&path)?
                {
                    modules.push(node);
                }
            }
        }

        let mut graph = DiGraph::new();
        let mut name_to_node = HashMap::new();

        for m in &modules {
            let idx = graph.add_node(m.clone());
            name_to_node.insert(m.name.clone(), idx);
        }

        for m in &modules {
            let dependent_idx = name_to_node[&m.name];

            // Implementation depends on its interface (.fsti)
            if !m.is_interface
                && let Some(&interface_idx) =
                    name_to_node.get(&format!("{}.interface", m.name))
            {
                graph.add_edge(interface_idx, dependent_idx, ());
            }

            // Implementation depends on opened/friended modules
            for imported in &m.opens {
                if let Some(&dep_idx) = name_to_node.get(imported)
                    && dep_idx != dependent_idx
                {
                    graph.add_edge(dep_idx, dependent_idx, ());
                }
            }
        }

        if let Err(cycle) = toposort(&graph, None) {
            let node = &graph[cycle.node_id()];
            return Err(GraphError::CycleDetected(node.name.clone()));
        }

        Ok(Self {
            graph,
            name_to_node,
        })
    }

    /// Fast scan of file header to extract `module <Name>` and `open <Name>` statements.
    pub fn parse_module_header(
        path: &Path,
    ) -> Result<Option<ModuleNode>, GraphError> {
        let content = fs::read_to_string(path)?;
        let mut module_name = None;
        let mut opens = HashSet::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("(*") {
                continue;
            }
            if trimmed.starts_with("module ")
                && !trimmed.contains('=')
                && module_name.is_none()
            {
                if let Some(name) = trimmed.split_whitespace().nth(1) {
                    module_name = Some(name.trim_end_matches(';').to_string());
                }
            } else if trimmed.starts_with("module ") && trimmed.contains('=') {
                // Track module aliases, e.g. `module Sem = Vale.X64.Semantics`
                let parts: Vec<&str> = trimmed.split('=').collect();
                if parts.len() == 2 {
                    let target = parts[1].trim().trim_end_matches(';').trim();
                    if !target.is_empty() {
                        opens.insert(target.to_string());
                    }
                }
            } else if (trimmed.starts_with("open ")
                || trimmed.starts_with("friend "))
                && let Some(target) = trimmed.split_whitespace().nth(1)
            {
                opens.insert(target.trim_end_matches(';').to_string());
            }
        }

        if let Some(name) = module_name {
            let is_interface =
                path.extension().and_then(|s| s.to_str()) == Some("fsti");
            let key = if is_interface {
                format!("{}.interface", name)
            } else {
                name
            };
            Ok(Some(ModuleNode {
                name: key,
                path: path.to_path_buf(),
                is_interface,
                opens: opens.into_iter().collect(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Returns topologically sorted list of modules for verification execution.
    pub fn execution_order(&self) -> Vec<ModuleNode> {
        toposort(&self.graph, None)
            .unwrap_or_default()
            .into_iter()
            .map(|idx| self.graph[idx].clone())
            .collect()
    }

    /// Computes parallel execution waves (levels) where modules in each wave are mutually independent.
    pub fn execution_batches(&self) -> Vec<Vec<ModuleNode>> {
        let mut in_degrees: HashMap<NodeIndex, usize> = HashMap::new();
        for node in self.graph.node_indices() {
            in_degrees.insert(
                node,
                self.graph
                    .neighbors_directed(node, petgraph::Direction::Incoming)
                    .count(),
            );
        }

        let mut batches = Vec::new();
        let mut processed = HashSet::new();

        loop {
            let ready: Vec<NodeIndex> = in_degrees
                .iter()
                .filter(|(n, deg)| **deg == 0 && !processed.contains(*n))
                .map(|(n, _)| *n)
                .collect();

            if ready.is_empty() {
                break;
            }

            let mut batch = Vec::new();
            for &n in &ready {
                processed.insert(n);
                batch.push(self.graph[n].clone());
                for neighbor in self
                    .graph
                    .neighbors_directed(n, petgraph::Direction::Outgoing)
                {
                    if let Some(deg) = in_degrees.get_mut(&neighbor)
                        && *deg > 0
                    {
                        *deg -= 1;
                    }
                }
            }
            batches.push(batch);
        }

        batches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_depgraph_dag_order() {
        let mut graph = DiGraph::new();
        let mut name_to_node = HashMap::new();

        let n1 = ModuleNode {
            name: "Spec.Core".to_string(),
            path: PathBuf::from("Spec.Core.fst"),
            is_interface: false,
            opens: vec![],
        };
        let n2 = ModuleNode {
            name: "Impl.Core".to_string(),
            path: PathBuf::from("Impl.Core.fst"),
            is_interface: false,
            opens: vec!["Spec.Core".to_string()],
        };

        let idx1 = graph.add_node(n1);
        let idx2 = graph.add_node(n2);
        name_to_node.insert("Spec.Core".to_string(), idx1);
        name_to_node.insert("Impl.Core".to_string(), idx2);
        graph.add_edge(idx1, idx2, ());

        let dg = DependencyGraph {
            graph,
            name_to_node,
        };
        let order = dg.execution_order();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0].name, "Spec.Core");
        assert_eq!(order[1].name, "Impl.Core");

        let batches = dg.execution_batches();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[0][0].name, "Spec.Core");
        assert_eq!(batches[1].len(), 1);
        assert_eq!(batches[1][0].name, "Impl.Core");
    }
}
