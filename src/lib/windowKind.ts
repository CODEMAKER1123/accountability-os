export type WindowKind = "popup" | "widget" | "capture" | null;

const QUERY_KINDS = new Set<Exclude<WindowKind, null>>(["popup", "widget", "capture"]);

/**
 * Tauri's App URL accepts only a path. Putting a query string in that path can
 * make the asset resolver request a file that does not exist, leaving an
 * auxiliary window completely blank. Production windows therefore route by
 * their stable Tauri label. The query-string fallback remains useful when a
 * surface is previewed in a normal browser during development.
 */
export function resolveWindowKind(
  search: string,
  tauriLabel?: string | null,
  hash = "",
): WindowKind {
  if (tauriLabel != null) {
    if (tauriLabel === "intervention") return "popup";
    if (tauriLabel === "widget") return "widget";
    if (tauriLabel === "capture") return "capture";
    return null;
  }

  const requested =
    new URLSearchParams(search).get("window") ??
    new URLSearchParams(hash.replace(/^#/, "")).get("window");
  return requested && QUERY_KINDS.has(requested as Exclude<WindowKind, null>)
    ? (requested as Exclude<WindowKind, null>)
    : null;
}
