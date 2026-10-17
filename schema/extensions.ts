import { z } from "zod/v4";
import { SensePlan } from "./signals.js";

const ExtensionId = z.string().regex(
  /^[a-z0-9][a-z0-9_-]*(?:\.[a-z0-9][a-z0-9_-]*)+$/,
  "Use a lowercase namespaced id such as user.my-provider",
);

const Version = z.string().regex(/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/);
const EnvironmentName = z.string().regex(/^[A-Za-z_][A-Za-z0-9_]*$/);

const CommandRuntime = z.object({
  type: z.literal("command"),
  command: z.array(z.string().min(1)).min(1),
  protocol: z.literal("video-perception.provider-stdio/v1"),
});

const CompatibleRuntime = z.object({
  type: z.enum(["openai-compatible", "ollama-compatible"]),
  base_url: z.string().url().optional(),
  base_url_env: EnvironmentName.optional(),
  credential_env: EnvironmentName.optional(),
}).refine(
  (runtime) => runtime.base_url !== undefined || runtime.base_url_env !== undefined,
  "A compatible runtime requires base_url or base_url_env",
);

const NativeRuntime = z.object({
  type: z.literal("native"),
});

export const ProviderRuntime = z.union([
  CommandRuntime,
  CompatibleRuntime,
  NativeRuntime,
]);

export const ProviderCapability = z.object({
  id: z.string().min(1),
  contract: z.string().min(1),
  modes: z.array(z.enum(["video", "streaming"])).min(1).default(["video"]),
  model: z.string().min(1).optional(),
  output_schema: z.string().min(1).optional(),
});

export const ProviderManifest = z.object({
  contract: z.literal("video-perception.provider/v1"),
  id: ExtensionId,
  version: Version,
  description: z.string().min(1),
  locality: z.enum(["local", "remote"]),
  media_transfer: z.enum(["never", "possible", "required"]),
  runtime: ProviderRuntime,
  capabilities: z.array(ProviderCapability).min(1),
  license: z.string().min(1).optional(),
}).refine(
  (provider) => provider.locality !== "remote" || provider.media_transfer !== "never",
  "A remote provider cannot declare media_transfer: never",
);

export const StrategyManifest = z.object({
  contract: z.literal("video-perception.strategy/v1"),
  id: ExtensionId,
  version: Version,
  description: z.string().min(1),
  plan: SensePlan,
  guidance: z.array(z.string().min(1)).default([]),
  license: z.string().min(1).optional(),
});

export const KnowledgePackManifest = z.object({
  contract: z.literal("video-perception.knowledge-pack/v1"),
  id: ExtensionId,
  version: Version,
  description: z.string().min(1),
  strategy: ExtensionId.optional(),
  knowledge: z.array(z.string().min(1)).min(1),
  license: z.string().min(1).optional(),
});

export type ProviderManifest = z.infer<typeof ProviderManifest>;
export type StrategyManifest = z.infer<typeof StrategyManifest>;
export type KnowledgePackManifest = z.infer<typeof KnowledgePackManifest>;
