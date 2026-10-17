# 0002: Keep investigation in the host, add bounded sampling

Status: accepted for source implementation, 2026-09-09.

The host agent owns planning, visual interpretation, follow-up questions, and
the final answer. The plugin supplies a coarse-to-fine observation method; the
CLI provides a deterministic `sample` primitive alongside unchanged `observe`
and `stream` contracts. No embedded autonomous planner is added.

A separate `video-perception.sample/v1` artifact avoids pretending a frame
selection is a full Observation or a semantic result. New directories retain
previous evidence. Parent links carry source identity and cumulative frame
budgets, not model viewing history. Selected images use decoded PTS rather
than nominal frame-rate arithmetic. Sampling is local-only, bounded by frame
count and wall time, with no automatic installation or provider escalation.

This method can help different host models use tools consistently; it cannot
confer another model's visual intelligence. Comparative quality and cost claims
require repeated same-model evaluations with and without the plugin.

Tradeoff: decoding from the beginning preserves a well-defined video time
origin but costs more for late windows. Seek optimization and audio timeline
reconciliation are separate work, not silently inferred here.
