type SearchableSession = {
  display_name: string;
  session_id: string;
  file_path: string;
};

export function sessionMatchesQuery(session: SearchableSession, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;

  return [
    session.display_name,
    session.session_id,
    session.file_path,
  ].some((value) => value.toLowerCase().includes(q));
}
