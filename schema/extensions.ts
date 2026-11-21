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

/** A tool the agent invokes itself; Video Perception never runs it. */
const ExternalRuntime = z.object({
  type: z.literal("external"),
});

export const ProviderRuntime = z.union([
  CommandRuntime,
  CompatibleRuntime,
  NativeRuntime,
  ExternalRuntime,
]);

/**
 * One read-only check. Exactly one of: `command` (optionally overridden by the
 * path in `env`), a file check (`env` and/or `file_globs`), or `python_module`
 * (a top-level package located with importlib find_spec, never imported).
 */
export const DetectCheck = z.object({
  id: z.string().min(1),
  detail: z.string().min(1),
  action: z.string().min(1).optional(),
  command: z.string().min(1).optional(),
  env: EnvironmentName.optional(),
  file_globs: z.array(z.string().min(1)).optional(),
  python_module: z.string().regex(/^[A-Za-z0-9_]+$/).optional(),
}).refine(
  (check) => {
    const isFile = check.command === undefined && (check.env !== undefined || (check.file_globs?.length ?? 0) > 0);
    const kinds = [check.command !== undefined, check.python_module !== undefined, isFile].filter(Boolean).length;
    return kinds === 1 && !(check.python_module !== undefined && check.env !== undefined);
  },
  "A check is exactly one of command, file (env/file_globs), or python_module",
);

/** Read-only checks that say whether a provider can run on the current machine. */
export const ProviderDetect = z.object({
  platforms: z.array(z.enum(["macos", "linux", "windows"])).optional(),
  checks: z.array(DetectCheck).default([]),
  verification: z.string().min(1).optional(),
  ready_status: z.string().min(1).optional(),
  missing_status: z.string().min(1).optional(),
});

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
  detect: ProviderDetect.optional(),
  usage: z.string().min(1).optional(),
  notes: z.array(z.string().min(1)).optional(),
}).refine(
  (provider) => provider.locality !== "remote" || provider.media_transfer !== "never",
  "A remote provider cannot declare media_transfer: never",
).refine(
  (provider) => provider.runtime.type !== "external" || provider.usage !== undefined,
  "An external provider must say how the agent invokes it in usage",
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
