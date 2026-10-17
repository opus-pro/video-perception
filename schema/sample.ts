import { z } from "zod/v4";

/** Local visual evidence; extraction is not a claim of semantic understanding. */
export const Sample = z.object({
  contract: z.literal("video-perception.sample/v1"),
  source: z.object({ path: z.string(), bytes: z.number().int().nonnegative(), modified_ns: z.string() }),
  timeline: z.string(),
  parent: z.string().nullable(),
  round: z.number().int().positive(),
  total_frames: z.number().int().nonnegative(),
  total_frame_budget: z.number().int().nonnegative(),
  requested: z.object({
    start_seconds: z.number().nonnegative(), end_seconds: z.number().positive(),
    fps: z.number().positive(), max_frames: z.number().int().positive(),
    max_size: z.number().int().positive(), crop: z.string().nullable(),
    timeout_seconds: z.number().int().positive(),
  }),
  provenance: z.object({
    provider: z.literal("local-ffmpeg"), locality: z.literal("local"),
    model: z.null(), filter: z.string(), decode_log: z.string(),
  }),
  status: z.enum(["sampled", "partial"]),
  warnings: z.array(z.string()),
  frames: z.array(z.object({ id: z.string(), path: z.string(), timestamp_seconds: z.number().nonnegative() })),
  sheet: z.string().nullable(),
  elapsed_ms: z.number().int().nonnegative(),
});
