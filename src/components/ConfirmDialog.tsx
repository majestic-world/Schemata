import { useEffect, useState } from "react";
import { Dialog } from "./Dialog";
import { useConfirm } from "../store/confirm";

export function ConfirmDialog() {
  const { request, settle } = useConfirm();
  const [typed, setTyped] = useState("");
  useEffect(() => setTyped(request?.input?.initial ?? ""), [request]);
  if (!request) return null;
  const blocked =
    (request.requireText !== undefined && typed !== request.requireText) || (request.input !== undefined && typed.trim() === "");
  const ok = () => !blocked && settle(request.input ? typed.trim() : "ok");
  return (
    <Dialog
      open
      onClose={() => settle(null)}
      title={request.title}
      width={420}
      footer={
        <>
          <button className="btn" onClick={() => settle(null)}>Cancel</button>
          <button className={`btn ${request.danger ? "danger solid" : "primary"}`} disabled={blocked} onClick={ok}>
            {request.confirmLabel}
          </button>
        </>
      }
    >
      {request.message && <p className="dialog-text">{request.message}</p>}
      {(request.requireText !== undefined || request.input) && (
        <form onSubmit={(e) => { e.preventDefault(); ok(); }}>
          <label className="field">
            <span>{request.input ? request.input.label : <>Type <b>{request.requireText}</b> to confirm</>}</span>
            <input autoFocus value={typed} onChange={(e) => setTyped(e.target.value)} />
          </label>
        </form>
      )}
    </Dialog>
  );
}
