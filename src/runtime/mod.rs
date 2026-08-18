pub mod arena;
pub mod budget;
pub mod governor;
pub mod lru;
pub mod pool;

pub use arena::{StringArena, StringRef};
pub use budget::{BudgetInfo, Subsystem, SubsystemBudgets};
pub use governor::{MemoryGovernor, MemoryPressure};
pub use lru::{LruCache, LruStats};
pub use pool::{BufferPool, PooledBuffer, SharedBufferPool};

#[derive(Default)]
pub struct VitaRuntime {
    pub governor: MemoryGovernor,
    pub budgets: SubsystemBudgets,
    pub pool: SharedBufferPool,
    pub arena: StringArena,
}

impl VitaRuntime {
    pub fn new() -> Self {
        Self {
            governor: MemoryGovernor::new(),
            budgets: SubsystemBudgets::default(),
            pool: SharedBufferPool::default(),
            arena: StringArena::new(),
        }
    }

    pub fn tick(&mut self) -> (MemoryPressure, bool) {
        self.governor.tick()
    }

    pub fn pressure(&self) -> MemoryPressure {
        self.governor.current_pressure()
    }

    pub fn free_memory_bytes(&self) -> usize {
        self.governor.last_free_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_initialization() {
        let mut runtime = VitaRuntime::new();
        let (pressure, _) = runtime.tick();
        assert_eq!(pressure, MemoryPressure::Normal);
        assert!(runtime.budgets.get(Subsystem::Textures).limit_bytes > 0);
    }
}
