import { useQuery } from "@tanstack/react-query";
import { Dialog } from "./Dialog";

/** The app's own license and the notices of every bundled third-party component, loaded only when opened. */
const loadLicenses = async () => {
  const [own, thirdParty] = await Promise.all([import("../../LICENSE?raw"), import("../../THIRD_PARTY_LICENSES.txt?raw")]);
  return `${own.default}\n\n${thirdParty.default}`;
};

export function LicensesDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const text = useQuery({ queryKey: ["licenses"], queryFn: loadLicenses, enabled: open, staleTime: Infinity });
  return (
    <Dialog open={open} onClose={onClose} title="Licenses" width={720} className="licenses-dialog" footer={<button className="btn" onClick={onClose}>Close</button>}>
      {text.isError ? (
        <p className="error-text">Could not load the license texts.</p>
      ) : (
        <pre className="licenses-text" tabIndex={0} aria-label="License texts">{text.data ?? "Loading…"}</pre>
      )}
    </Dialog>
  );
}
