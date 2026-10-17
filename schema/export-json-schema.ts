/**
 * Export Zod schemas as JSON Schema files.
 * Usage: npx tsx schema/export-json-schema.ts
 * Output: schema/json/ directory with .schema.json files
 */
import { z } from "zod/v4";
import { writeFileSync, mkdirSync } from "fs";
import { join } from "path";
import {
  Metadata, Scenes, SceneStats, Audio, SceneAudio, Beats, Transcript,
  ObservationManifest, SensePlan, SignalDeck,
} from "./signals.js";
import { StreamEvent, StreamSession } from "./stream.js";
import { Sample } from "./sample.js";
import { KnowledgePackManifest, ProviderManifest, StrategyManifest } from "./extensions.js";

const outDir = join(process.cwd(), "schema", "json");
mkdirSync(outDir, { recursive: true });

const schemas: Record<string, z.ZodType> = {
  sample: Sample,
  metadata: Metadata,
  scenes: Scenes,
  scene_stats: SceneStats,
  audio: Audio,
  scene_audio: SceneAudio,
  beats: Beats,
  transcript: Transcript,
  sense_plan: SensePlan,
  observation_manifest: ObservationManifest,
  signal_deck: SignalDeck,
  stream_event: StreamEvent,
  stream_session: StreamSession,
  provider_manifest: ProviderManifest,
  strategy_manifest: StrategyManifest,
  knowledge_pack_manifest: KnowledgePackManifest,
};

for (const [name, schema] of Object.entries(schemas)) {
  const jsonSchema = z.toJSONSchema(schema, { target: "draft-2020-12" });
  const path = join(outDir, `${name}.schema.json`);
  writeFileSync(path, JSON.stringify(jsonSchema, null, 2) + "\n");
  console.log(`  ${name} → ${path}`);
}

console.log(`\nExported ${Object.keys(schemas).length} schemas.`);
