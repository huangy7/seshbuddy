import { invokeApp } from "../utils/invokeApp";
import type { SessionIdentity } from "../types/session";

export function renameSession(identity: SessionIdentity, newName: string) {
  return invokeApp("rename_session", {
    cliId: identity.cliId,
    filePath: identity.filePath,
    newName,
  });
}
