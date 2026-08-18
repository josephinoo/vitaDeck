pub const HEAP_BUDGET_BYTES: usize = 192 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryPressure {
    Normal,     
    Warning,    
    Aggressive, 
    Emergency,  
}

impl MemoryPressure {
    pub fn label(&self) -> &'static str {
        match self {
            MemoryPressure::Normal => "NORMAL",
            MemoryPressure::Warning => "WARNING",
            MemoryPressure::Aggressive => "AGGRESSIVE EVICTION",
            MemoryPressure::Emergency => "EMERGENCY",
        }
    }
}

pub struct MemoryGovernor {
    current_pressure: MemoryPressure,
    last_free_bytes: usize,
    simulated_free_bytes: Option<usize>,
    normal_threshold_bytes: usize,
    warning_threshold_bytes: usize,
    aggressive_threshold_bytes: usize,
}

impl Default for MemoryGovernor {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryGovernor {
    pub fn new() -> Self {
        Self {
            current_pressure: MemoryPressure::Normal,
            last_free_bytes: HEAP_BUDGET_BYTES,
            simulated_free_bytes: None,
            normal_threshold_bytes: HEAP_BUDGET_BYTES / 2,     
            warning_threshold_bytes: HEAP_BUDGET_BYTES / 4,    
            aggressive_threshold_bytes: HEAP_BUDGET_BYTES / 8, 
        }
    }

    pub fn current_pressure(&self) -> MemoryPressure {
        self.current_pressure
    }

    pub fn last_free_bytes(&self) -> usize {
        self.last_free_bytes
    }

    pub fn set_simulated_free_bytes(&mut self, free_bytes: Option<usize>) {
        self.simulated_free_bytes = free_bytes;
    }

    pub fn poll_system_free_memory(&self) -> usize {
        if let Some(simulated) = self.simulated_free_bytes {
            return simulated;
        }

        #[cfg(target_os = "vita")]
        {
            use core::ffi::c_int;
            unsafe extern "C" {
                fn mallinfo() -> MallInfo;
            }
            #[repr(C)]
            struct MallInfo {
                arena: c_int,
                ordblks: c_int,
                smblks: c_int,
                hblks: c_int,
                hblkhd: c_int,
                usmblks: c_int,
                fsmblks: c_int,
                uordblks: c_int,
                fordblks: c_int,
                keepcost: c_int,
            }

            unsafe {
                let info = mallinfo();
                let live = info.uordblks.max(0) as usize;
                HEAP_BUDGET_BYTES.saturating_sub(live)
            }
        }

        #[cfg(not(target_os = "vita"))]
        {
            HEAP_BUDGET_BYTES
        }
    }

    pub fn tick(&mut self) -> (MemoryPressure, bool) {
        let free_bytes = self.poll_system_free_memory();
        self.last_free_bytes = free_bytes;

        let new_pressure = if free_bytes >= self.normal_threshold_bytes {
            MemoryPressure::Normal
        } else if free_bytes >= self.warning_threshold_bytes {
            MemoryPressure::Warning
        } else if free_bytes >= self.aggressive_threshold_bytes {
            MemoryPressure::Aggressive
        } else {
            MemoryPressure::Emergency
        };

        let changed = new_pressure != self.current_pressure;
        self.current_pressure = new_pressure;
        (new_pressure, changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_pressure_transitions() {
        let mut gov = MemoryGovernor::new();

        gov.set_simulated_free_bytes(Some(150 * 1024 * 1024));
        let (pressure, _changed) = gov.tick();
        assert_eq!(pressure, MemoryPressure::Normal);

        gov.set_simulated_free_bytes(Some(60 * 1024 * 1024));
        let (pressure, changed) = gov.tick();
        assert_eq!(pressure, MemoryPressure::Warning);
        assert!(changed);

        gov.set_simulated_free_bytes(Some(30 * 1024 * 1024));
        let (pressure, changed) = gov.tick();
        assert_eq!(pressure, MemoryPressure::Aggressive);
        assert!(changed);

        gov.set_simulated_free_bytes(Some(15 * 1024 * 1024));
        let (pressure, changed) = gov.tick();
        assert_eq!(pressure, MemoryPressure::Emergency);
        assert!(changed);
    }
}
