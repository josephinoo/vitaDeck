#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subsystem {
    Textures,
    Metadata,
    Audio,
    Ui,
    Network,
    Other,
}

impl Subsystem {
    pub fn name(&self) -> &'static str {
        match self {
            Subsystem::Textures => "Textures",
            Subsystem::Metadata => "Metadata",
            Subsystem::Audio => "Audio",
            Subsystem::Ui => "UI & Meshes",
            Subsystem::Network => "Network",
            Subsystem::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BudgetInfo {
    pub limit_bytes: usize,
    pub used_bytes: usize,
    pub peak_bytes: usize,
}

impl BudgetInfo {
    pub fn new(limit_bytes: usize) -> Self {
        Self {
            limit_bytes,
            used_bytes: 0,
            peak_bytes: 0,
        }
    }

    pub fn record_alloc(&mut self, bytes: usize) {
        self.used_bytes += bytes;
        if self.used_bytes > self.peak_bytes {
            self.peak_bytes = self.used_bytes;
        }
    }

    pub fn record_free(&mut self, bytes: usize) {
        self.used_bytes = self.used_bytes.saturating_sub(bytes);
    }

    pub fn has_headroom(&self, required: usize) -> bool {
        self.used_bytes + required <= self.limit_bytes
    }

    pub fn usage_fraction(&self) -> f32 {
        if self.limit_bytes == 0 {
            0.0
        } else {
            (self.used_bytes as f32 / self.limit_bytes as f32).min(1.0)
        }
    }

    pub fn remaining_bytes(&self) -> usize {
        self.limit_bytes.saturating_sub(self.used_bytes)
    }
}

#[derive(Debug, Clone)]
pub struct SubsystemBudgets {
    pub textures: BudgetInfo,
    pub metadata: BudgetInfo,
    pub audio: BudgetInfo,
    pub ui: BudgetInfo,
    pub network: BudgetInfo,
    pub other: BudgetInfo,
}

impl Default for SubsystemBudgets {
    fn default() -> Self {
        Self {
            textures: BudgetInfo::new(60 * 1024 * 1024),   
            metadata: BudgetInfo::new(15 * 1024 * 1024),   
            audio: BudgetInfo::new(12 * 1024 * 1024),      
            ui: BudgetInfo::new(25 * 1024 * 1024),         
            network: BudgetInfo::new(8 * 1024 * 1024),     
            other: BudgetInfo::new(10 * 1024 * 1024),      
        }
    }
}

impl SubsystemBudgets {
    pub fn get(&self, sys: Subsystem) -> &BudgetInfo {
        match sys {
            Subsystem::Textures => &self.textures,
            Subsystem::Metadata => &self.metadata,
            Subsystem::Audio => &self.audio,
            Subsystem::Ui => &self.ui,
            Subsystem::Network => &self.network,
            Subsystem::Other => &self.other,
        }
    }

    pub fn get_mut(&mut self, sys: Subsystem) -> &mut BudgetInfo {
        match sys {
            Subsystem::Textures => &mut self.textures,
            Subsystem::Metadata => &mut self.metadata,
            Subsystem::Audio => &mut self.audio,
            Subsystem::Ui => &mut self.ui,
            Subsystem::Network => &mut self.network,
            Subsystem::Other => &mut self.other,
        }
    }

    pub fn record_alloc(&mut self, sys: Subsystem, bytes: usize) {
        self.get_mut(sys).record_alloc(bytes);
    }

    pub fn record_free(&mut self, sys: Subsystem, bytes: usize) {
        self.get_mut(sys).record_free(bytes);
    }

    pub fn total_used_bytes(&self) -> usize {
        self.textures.used_bytes
            + self.metadata.used_bytes
            + self.audio.used_bytes
            + self.ui.used_bytes
            + self.network.used_bytes
            + self.other.used_bytes
    }

    pub fn total_limit_bytes(&self) -> usize {
        self.textures.limit_bytes
            + self.metadata.limit_bytes
            + self.audio.limit_bytes
            + self.ui.limit_bytes
            + self.network.limit_bytes
            + self.other.limit_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_tracking() {
        let mut budgets = SubsystemBudgets::default();
        assert!(budgets.get(Subsystem::Textures).has_headroom(10 * 1024 * 1024));

        budgets.record_alloc(Subsystem::Textures, 50 * 1024 * 1024);
        assert_eq!(budgets.get(Subsystem::Textures).used_bytes, 50 * 1024 * 1024);
        assert_eq!(budgets.get(Subsystem::Textures).peak_bytes, 50 * 1024 * 1024);
        assert!(!budgets.get(Subsystem::Textures).has_headroom(15 * 1024 * 1024));

        budgets.record_free(Subsystem::Textures, 20 * 1024 * 1024);
        assert_eq!(budgets.get(Subsystem::Textures).used_bytes, 30 * 1024 * 1024);
        assert_eq!(budgets.get(Subsystem::Textures).peak_bytes, 50 * 1024 * 1024);
    }
}
