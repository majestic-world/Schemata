import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App";
import { useAppearance } from "./store/appearance";
import "./styles/tokens.css";
import "./styles/app.css";

// Apply the persisted palette before React paints, including portal-based dialogs.
document.documentElement.dataset.theme = useAppearance.getState().theme;
useAppearance.subscribe(({ theme }) => {
  document.documentElement.dataset.theme = theme;
});

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false, staleTime: 30_000 } },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
