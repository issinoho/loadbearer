//! Scoring profiles: how much each component counts toward the overall grade.
//! "Which machine is better" depends on the intended use, so the overall is a
//! profile-weighted geometric mean of the component scores.

#[derive(Debug, Clone, Copy)]
pub struct Profile {
    pub name: &'static str,
    pub description: &'static str,
    /// (component id, weight). Components not listed default to weight 1.0.
    weights: &'static [(&'static str, f64)],
}

impl Profile {
    /// Weight for a component id (1.0 if the profile does not mention it).
    pub fn weight(&self, component_id: &str) -> f64 {
        self.weights
            .iter()
            .find(|(id, _)| *id == component_id)
            .map_or(1.0, |(_, w)| *w)
    }
}

// Weights apply to the graded components (CPU, memory, disk). The network
// component is scored and shown but not folded into the overall, so it carries
// no weight here.
pub const GENERAL: Profile = Profile {
    name: "general",
    description: "balanced — every component counts equally",
    weights: &[("cpu", 1.0), ("memory", 1.0), ("disk", 1.0)],
};

pub const DEV_WORKSTATION: Profile = Profile {
    name: "dev-workstation",
    description: "favours CPU and disk (builds, containers, VCS)",
    weights: &[("cpu", 1.4), ("memory", 1.0), ("disk", 1.3)],
};

pub const CONTENT_CREATION: Profile = Profile {
    name: "content-creation",
    description: "favours CPU and memory bandwidth (encode, render)",
    weights: &[("cpu", 1.4), ("memory", 1.3), ("disk", 0.8)],
};

pub const SERVER: Profile = Profile {
    name: "server",
    description: "favours disk I/O and CPU (sustained throughput under load)",
    weights: &[("cpu", 1.3), ("memory", 1.0), ("disk", 1.4)],
};

pub const ALL: &[Profile] = &[GENERAL, DEV_WORKSTATION, CONTENT_CREATION, SERVER];

pub fn by_name(name: &str) -> Option<Profile> {
    ALL.iter().find(|p| p.name == name).copied()
}

pub fn names() -> Vec<&'static str> {
    ALL.iter().map(|p| p.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_is_uniform() {
        for c in ["cpu", "memory", "disk", "network"] {
            assert_eq!(GENERAL.weight(c), 1.0);
        }
    }

    #[test]
    fn unknown_component_defaults_to_one() {
        assert_eq!(DEV_WORKSTATION.weight("gpu"), 1.0);
    }

    #[test]
    fn lookup_by_name() {
        assert_eq!(by_name("server").unwrap().name, "server");
        assert!(by_name("bogus").is_none());
    }
}
