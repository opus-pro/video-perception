/** Stable contracts produced by `video-perception stream`. */
import { z } from "zod/v4";
import { SensePlan } from "./signals.js";

const ms = z.number().int().nonnegative();

export const StreamProvider = z.object({
  id: z.string().min(1),
  class: z.enum([
    "built-in-local",
    "system-native",
    "user-managed-local",
    "user-configured-remote",
  ]),
  locality: z.enum(["local", "remote"]),
});

export const StreamEvent = z.object({
  contract: z.literal("video-perception.stream-event/v1"),
  session_id: z.string().min(1),
  seq: z.number().int().positive(),
  observed_at_unix_ms: ms,
  media_time_ms: ms.optional(),
  capability: z.string().min(1),
  kind: z.enum([
    "session.started",
    "session.finalizing",
    "session.closed",
    "measurement",
    "asset",
    "capability.completed",
    "capability.failed",
    "discontinuity",
  ]),
  status: z.enum(["provisional", "final", "error"]),
  provider: StreamProvider.optional(),
  payload: z.record(z.string(), z.unknown()),
});

export const StreamSessionCapability = z.object({
  id: z.string().min(1),
  required: z.boolean(),
  strategy: z.enum(["native", "windowed", "finalize-only", "unsupported"]),
  provider: StreamProvider,
  status: z.enum(["running", "succeeded", "failed"]),
  dropped_events: z.number().int().nonnegative(),
  summary: z.record(z.string(), z.unknown()).optional(),
  error: z.string().min(1).optional(),
});

export const StreamSession = z.object({
  contract: z.literal("video-perception.stream-session/v1"),
  version: z.literal("1.0"),
  session_id: z.string().min(1),
  state: z.enum(["running", "closed", "failed", "cancelled"]),
  source: z.object({
    uri: z.string().min(1),
    kind: z.enum(["file", "network"]),
    pacing: z.enum(["readrate", "unpaced", "source-paced"]),
    speed: z.number().nonnegative(),
  }),
  sense_plan: SensePlan,
  started_at_unix_ms: ms,
  ended_at_unix_ms: ms.optional(),
  event_count: z.number().int().nonnegative(),
  backpressure: z.enum(["block", "drop-newest"]),
  event_buffer: z.number().int().positive(),
  capabilities: z.array(StreamSessionCapability).min(1),
  files: z.object({
    events: z.literal("events.jsonl"),
    tracks: z.literal("tracks/"),
    observation: z.literal("observation").optional(),
  }),
});

export type StreamProvider = z.infer<typeof StreamProvider>;
export type StreamEvent = z.infer<typeof StreamEvent>;
export type StreamSessionCapability = z.infer<typeof StreamSessionCapability>;
export type StreamSession = z.infer<typeof StreamSession>;
