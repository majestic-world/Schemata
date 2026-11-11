import { X, AlertCircle, CheckCircle2, Info } from "lucide-react";
import { useToasts } from "../store/toasts";

export function Toaster() {
  const { toasts, dismiss } = useToasts();
  return (
    <div className="toaster" role="status">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.tone}`}>
          {t.tone === "error" ? <AlertCircle size={18} /> : t.tone === "success" ? <CheckCircle2 size={18} /> : <Info size={18} />}
          <div className="toast-text">
            <b>{t.title}</b>
            {t.message && <span>{t.message}</span>}
          </div>
          <button className="icon-btn" onClick={() => dismiss(t.id)} aria-label="Dismiss"><X size={14} /></button>
        </div>
      ))}
    </div>
  );
}
