use anyhow::{Context, Result, bail};
use peasy_core::{EngineDecision, EngineInput};
use std::path::Path;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

struct HostState {
    limits: StoreLimits,
}

pub struct EngineHost {
    engine: Engine,
    module: Module,
}

impl EngineHost {
    pub fn load(path: &Path) -> Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let module = Module::from_file(&engine, path)
            .map_err(|error| anyhow::anyhow!("loading engine {}: {error}", path.display()))?;
        if let Some(import) = module.imports().next() {
            bail!(
                "engine imports forbidden capability {}::{}",
                import.module(),
                import.name()
            );
        }
        Ok(Self { engine, module })
    }

    pub fn resolve(&self, input: &EngineInput) -> Result<EngineDecision> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(16 * 1024 * 1024)
            .instances(1)
            .memories(1)
            .build();
        let mut store = Store::new(&self.engine, HostState { limits });
        store.limiter(|state| &mut state.limits);
        store.set_fuel(2_000_000)?;
        let linker = Linker::new(&self.engine);
        let instance = linker.instantiate(&mut store, &self.module)?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("engine has no exported memory")?;
        let alloc = instance.get_typed_func::<u32, u32>(&mut store, "peasy_alloc")?;
        let resolve = instance.get_typed_func::<(u32, u32), u64>(&mut store, "peasy_resolve")?;
        let dealloc = instance.get_typed_func::<(u32, u32), ()>(&mut store, "peasy_dealloc")?;

        let encoded = serde_json::to_vec(input)?;
        let input_len = u32::try_from(encoded.len()).context("engine input is too large")?;
        let input_ptr = alloc.call(&mut store, input_len)?;
        memory.write(&mut store, input_ptr as usize, &encoded)?;
        let packed = resolve.call(&mut store, (input_ptr, input_len))?;
        dealloc.call(&mut store, (input_ptr, input_len))?;

        let output_ptr = (packed >> 32) as u32;
        let output_len = packed as u32;
        if output_len > 64 * 1024 {
            bail!("engine returned an oversized response");
        }
        let mut output = vec![0; output_len as usize];
        memory.read(&store, output_ptr as usize, &mut output)?;
        dealloc.call(&mut store, (output_ptr, output_len))?;
        Ok(serde_json::from_slice(&output)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_wasi_imports_cannot_link() {
        let hostile = wat::parse_str(
            r#"(module
              (import "wasi_snapshot_preview1" "path_open" (func))
              (import "wasi_snapshot_preview1" "environ_get" (func))
              (import "wasi_snapshot_preview1" "sock_open" (func))
              (import "wasi_snapshot_preview1" "proc_exit" (func))
            )"#,
        )
        .unwrap();
        let engine = Engine::default();
        let module = Module::new(&engine, hostile).unwrap();
        let linker = Linker::<()>::new(&engine);
        let mut store = Store::new(&engine, ());
        assert!(linker.instantiate(&mut store, &module).is_err());
    }
}
