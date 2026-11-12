import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App";
import { useAppearance, type Theme } from "./store/appearance";
import { api } from "./api/client";
import "./styles/tokens.css";
import "./styles/app.css";

// Apply the persisted palette before React paints, including portal-based dialogs, and match the window frame to it.
const applyTheme = (theme: Theme) => {
  document.documentElement.dataset.theme = theme;
  void api.setWindowTheme(theme === "dark");
};
applyTheme(useAppearance.getState().theme);
useAppearance.subscribe(({ theme }) => applyTheme(theme));

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
