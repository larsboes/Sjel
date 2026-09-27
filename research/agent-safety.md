# Can an assistant act on a person's data safely?

First written 2026-09-27.

## What the sources show

**Agents fail often, even without an attacker.** AgentDojo has 97 realistic tasks and 629
security test cases. Its authors report that state-of-the-art models "fail at many tasks (even in
the absence of attacks)"
([Debenedetti et al., AgentDojo, arXiv:2406.13352](https://arxiv.org/abs/2406.13352)).

**The same request does not fail the same way twice.** In τ-bench, GPT-4o-class agents succeed on
fewer than 50% of tasks, and on fewer than 25% of retail tasks across all of eight repeated runs
([Yao, Shinn, Razavi and Narasimhan, τ-bench, arXiv:2406.12045](https://arxiv.org/abs/2406.12045)).

**Text an agent reads can steer it.** In 1,054 test cases, a ReAct-prompted GPT-4 followed
instructions injected into tool output 24% of the time
([Zhan, Liang, Ying and Kang, InjecAgent, ACL 2024 Findings, arXiv:2403.02691](https://arxiv.org/abs/2403.02691)).
Calendar invites, mail and shared documents are that kind of tool output.

**Risky actions happen at a measurable rate.** In an emulated sandbox, "even the safest LM agent"
showed risky failures 23.9% of the time, and human reviewers judged 68.8% of the flagged failures
as valid real-world risks
([Ruan et al., ToolEmu, arXiv:2309.15817](https://arxiv.org/abs/2309.15817)).

**Separating control from data works, at a cost.** CaMeL keeps the plan apart from untrusted data
and solves 77% of AgentDojo tasks with provable security, against 84% with no defence
([Debenedetti et al., Defeating Prompt Injections by Design, arXiv:2503.18813](https://arxiv.org/abs/2503.18813)).

## Open challenge

**Frequent confirmations get clicked through.** Chrome users clicked through 70.2% of SSL warnings
([Akhawe and Felt, Alice in Warningland, USENIX Security 2013](https://www.usenix.org/conference/usenixsecurity13/technical-sessions/presentation/akhawe)).
A confirmation protects only while it is rare.

## What the sources do not show

- None of these benchmarks uses household data or typed tools like Sjel's.
