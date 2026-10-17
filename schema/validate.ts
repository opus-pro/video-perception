/**
 * Validate a JSON file against a schema.
 * Usage: npx tsx schema/validate.ts <schema-name> <file.json>
 *
 * Schema names: metadata, scenes, scene_stats, audio, scene_audio, beats,
 * transcript, sense_plan, observation_manifest, signal_deck, stream_event,
 * stream_session, provider_manifest, strategy_manifest,
 * knowledge_pack_manifest
 */
import { z } from "zod/v4";
import { readFileSync } from "fs";
import {
  Metadata, Scenes, SceneStats, Audio, SceneAudio, Beats, Transcript,
  ObservationManifest, SensePlan, SignalDeck,
} from "./signals.js";
import { StreamEvent, StreamSession } from "./stream.js";
import { KnowledgePackManifest, ProviderManifest, StrategyManifest } from "./extensions.js";

const schemas: Record<string, z.ZodType> = {
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

const [schemaName, filePath] = process.argv.slice(2);

if (!schemaName || !filePath) {
  console.error(`Usage: npx tsx schema/validate.ts <${Object.keys(schemas).join("|")}> <file.json>`);
  process.exit(1);
}

const schema = schemas[schemaName];
if (!schema) {
  console.error(`Unknown schema: ${schemaName}. Available: ${Object.keys(schemas).join(", ")}`);
  process.exit(1);
}

const raw = JSON.parse(readFileSync(filePath, "utf-8"));
const result = schema.safeParse(raw);

if (result.success) {
  console.log(`✓ ${filePath} is valid ${schemaName}`);
} else {
  console.error(`✗ ${filePath} failed ${schemaName} validation:\n`);
  console.error(z.prettifyError(result.error));
  process.exit(1);
}
