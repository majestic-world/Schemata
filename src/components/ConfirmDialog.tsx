import { useEffect, useState } from "react";
import { Dialog } from "./Dialog";
import { useConfirm } from "../store/confirm";

export function ConfirmDialog() {
  const { request, settle } = useConfirm();
  const [typed, setTyped] = useState("");
  useEffect(() => setTyped(""), [request]);
  if (!request) return null;
  const blocked = request.requireText !== undefined && typed !== request.requireText;
  return (
    <Dialog
      open
      onClose={() => settle(false)}
      title={request.title}
      width={420}
      footer={
        <>
          <button className="btn" onClick={() => settle(false)}>Cancel</button>
          <button className={`btn ${request.danger ? "danger solid" : "primary"}`} disabled={blocked} onClick={() => settle(true)}>
            {request.confirmLabel}
          </button>
        </>
      }
    >
      <p className="dialog-text">{request.message}</p>
      {request.requireText !== undefined && (
        <label className="field">
          <span>Type <b>{request.requireText}</b> to confirm</span>
          <input autoFocus value={typed} onChange={(e) => setTyped(e.target.value)} />
        </label>
      )}
    </Dialog>
  );
}
