use std::collections::HashMap;

struct CountingProvider {
    calls: u32,
}

#[derive(Debug, Clone)]
struct Inference {
    value: String,
}

impl CountingProvider {
    fn new() -> Self {
        Self { calls: 0 }
    }

    fn infer(&mut self) {
        self.calls += 1;
    }

    fn cost(&self) -> u32 {
        self.calls * 2
    }
    fn infer_context(&mut self, context: &str) -> Inference {
        self.calls += 1;

        Inference {
            value: format!("Inference for {context}"),
        }
    }
}

fn main() {
    // 2. Scenario 1: Per-Agent Calls (No Sharing)
    let mut per_agent = CountingProvider::new();

    per_agent.infer(); // Agent 1
    per_agent.infer(); // Agent 2
    per_agent.infer(); // Agent 3

    println!(
        "per-agent calls={} cost={}",
        per_agent.calls,
        per_agent.cost()
    );

    // 3. Scenario 2: Shared Inference
    let mut shared = CountingProvider::new();

    // All three agents use the result of this one inference.
    shared.infer();

    println!("shared calls={} cost={}", shared.calls, shared.cost());

    // 4. Scenario 3: Share by Context

    let mut provider = CountingProvider::new();

    let mut cache: HashMap<String, Inference> = HashMap::new();

    let agent_contexts = ["Context A", "Context A", "Context B"];

    for context in agent_contexts {
        let inference = cache
            .entry(context.to_string())
            .or_insert_with(|| provider.infer_context(context));

        println!("{context}: {}", inference.value);
    }

    println!(
        "context-shared calls={} cost={}",
        provider.calls,
        provider.cost()
    );
}
