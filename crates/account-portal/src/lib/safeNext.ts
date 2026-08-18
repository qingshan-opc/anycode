/** Keep in sync with `crates/account-service/src/hop.rs::safe_next`. */
export const DEFAULT_APP_NEXT = "/console";

export function isSafeAppNext(raw: string): boolean {
  if (
    !raw.startsWith("/") ||
    raw.startsWith("//") ||
    raw.includes("\n") ||
    raw.includes("\r") ||
    raw.includes("..")
  ) {
    return false;
  }
  return (
    raw === "/m" ||
    raw.startsWith("/m/") ||
    raw.startsWith("/m?") ||
    raw === "/console" ||
    raw.startsWith("/console/") ||
    raw.startsWith("/console?")
  );
}

export function safeAppNext(raw: string | null | undefined): string {
  const trimmed = raw?.trim() ?? "";
  if (!trimmed) return DEFAULT_APP_NEXT;
  return isSafeAppNext(trimmed) ? trimmed : DEFAULT_APP_NEXT;
}
