import type { ConfiguredModel, ModelsRegistryView } from "@/api/types";

/**
 * Registry is the sole authority on modality support (roadmap P1.1) — the old
 * substring mirror of the Rust heuristic was removed; unknown models simply
 * fall through to the backend's authoritative re-check on send.
 */
function itemSupportsVision(item: ConfiguredModel): boolean {
  return item.capabilities.includes("vision");
}

/**
 * Whether the **active chat** model natively accepts multimodal image input.
 * Text-only brains (e.g. DeepSeek Flash) return false — use OCR fallback instead.
 */
export function chatModelSupportsVision(registry?: ModelsRegistryView | null): boolean {
  if (!registry) return true;
  const activeId = registry.active?.chat?.trim();
  const activeItem = activeId
    ? registry.items.find((m) => m.id === activeId)
    : undefined;
  if (activeItem) {
    return itemSupportsVision(activeItem);
  }
  const globalCandidate = registry.items.find(
    (m) =>
      m.enabled &&
      m.capabilities.includes("chat") &&
      m.provider === registry.global?.provider &&
      m.model === registry.global?.model,
  );
  if (globalCandidate) {
    return itemSupportsVision(globalCandidate);
  }
  // No resolved chat model yet — allow UI attach; backend re-checks on send.
  return true;
}

export type ImageAttachMediaStatus = {
  ocr_available?: boolean;
  image_attach_ok?: boolean;
  chat_supports_vision?: boolean;
  apple_media?: { ocr?: boolean } | null;
};

/**
 * Whether the composer may accept image paste/attach.
 * Native vision chat OR delegated OCR capability (brain + OCR).
 * When media status has loaded, honor `image_attach_ok === false` (no FE heuristic override).
 */
export function imageAttachAllowed(
  registry?: ModelsRegistryView | null,
  media?: ImageAttachMediaStatus | null,
): boolean {
  if (media?.image_attach_ok === true) return true;
  // Explicit BE deny after media status loaded — do not fall back to chat heuristics.
  if (media?.image_attach_ok === false) return false;
  if (media?.ocr_available === true) return true;
  if (media?.apple_media?.ocr === true) return true;
  return chatModelSupportsVision(registry);
}

export type ComposerModelOption = {
  id: string;
  label: string;
  subtitle: string;
  isCloud?: boolean;
  cloudModel?: string;
  /** Lifecycle tier: current | legacy | deprecated | removed (absent = current). */
  tier?: string;
  tierNote?: string | null;
  tierReplacement?: string | null;
  curated?: boolean;
};

export const COMPOSER_MODEL_STORAGE_KEY = "anycode-composer-model";
export const COMPOSER_AUTO_STORAGE_KEY = "anycode-composer-auto";

export function modelLabel(item: ConfiguredModel): string {
  return item.display_name?.trim() || `${item.provider}/${item.model}`;
}

export function modelSubtitle(item: ConfiguredModel): string {
  const providerModel = `${item.provider}/${item.model}`;
  if (item.display_name?.trim()) {
    return providerModel;
  }
  return item.provider;
}

export type ListChatModelsOptions = {
  /** When false, hide `source: cloud` / anycode_cloud entries (logged-out UI). Default true. */
  includeCloud?: boolean;
};

function isCloudConfiguredModel(item: ConfiguredModel): boolean {
  if (item.source === "cloud") return true;
  const provider = item.provider.trim().toLowerCase().replace(/-/g, "_");
  return provider === "anycode_cloud" || provider === "anycodecloud";
}

export function listChatModels(
  items: ConfiguredModel[],
  options: ListChatModelsOptions = {},
): ComposerModelOption[] {
  const includeCloud = options.includeCloud !== false;
  const seen = new Set<string>();
  const optionsList = items
    .filter((m) => m.enabled && m.capabilities.includes("chat"))
    .filter((m) => includeCloud || !isCloudConfiguredModel(m))
    .filter((m) => {
      const key = `${m.provider}/${m.model}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .map((item) => ({
      id: item.id,
      label: modelLabel(item),
      subtitle: modelSubtitle(item),
      isCloud: isCloudConfiguredModel(item),
      cloudModel: item.model,
      tier: item.tier,
      tierNote: item.tier_note,
      tierReplacement: item.tier_replacement,
      curated: item.curated,
    }));

  return optionsList.sort((a, b) => {
    const rank = (o: ComposerModelOption) => {
      if (o.isCloud && o.cloudModel === "auto") return 0;
      if (o.isCloud) return 1;
      // Outdated models sink below everything else; they stay selectable for
      // users who already rely on them.
      if (o.tier === "deprecated" || o.tier === "removed") return 4;
      if (o.curated) return 2;
      return 3;
    };
    const dr = rank(a) - rank(b);
    if (dr !== 0) return dr;
    return a.label.localeCompare(b.label, undefined, { sensitivity: "base" });
  });
}

export function readStoredModelId(): string | null {
  try {
    const v = localStorage.getItem(COMPOSER_MODEL_STORAGE_KEY);
    return v?.trim() || null;
  } catch {
    return null;
  }
}

/** Registry item id for global provider/model, if present in chat-capable items. */
export function findGlobalDefaultChatId(registry?: ModelsRegistryView | null): string | null {
  if (!registry) return null;
  const provider = registry.global?.provider?.trim();
  const model = registry.global?.model?.trim();
  if (!provider || !model) return registry.active?.chat ?? null;
  const match = registry.items.find(
    (item) =>
      item.enabled &&
      item.capabilities.includes("chat") &&
      item.provider === provider &&
      item.model === model,
  );
  return match?.id ?? registry.active?.chat ?? null;
}

export function readStoredAuto(fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(COMPOSER_AUTO_STORAGE_KEY);
    if (v === "1") return true;
    if (v === "0") return false;
  } catch {
    /* ignore */
  }
  return fallback;
}

export function writeStoredAuto(auto: boolean): void {
  try {
    localStorage.setItem(COMPOSER_AUTO_STORAGE_KEY, auto ? "1" : "0");
  } catch {
    /* ignore */
  }
}

export function writeStoredModelId(id: string): void {
  try {
    localStorage.setItem(COMPOSER_MODEL_STORAGE_KEY, id);
  } catch {
    /* ignore */
  }
}

/** Auto = active chat matches global default (routing picks per agent/mode). */
export function inferAutoFromRegistry(registry?: ModelsRegistryView | null): boolean {
  if (!registry) return true;
  const activeChat = registry.active?.chat;
  if (!activeChat) return true;
  const globalId = findGlobalDefaultChatId(registry);
  if (!globalId) return false;
  return activeChat === globalId;
}
