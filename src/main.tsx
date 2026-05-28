import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import "@/i18n";
import { loadTheme } from "@/lib/project-store";

// Apply theme before render to avoid flash
async function initApp() {
  const root = document.documentElement;

  // Try to load saved theme
  try {
    const savedTheme = await loadTheme();
    if (savedTheme) {
      if (savedTheme === "system") {
        // Follow system preference
        if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
          root.classList.add("dark");
        } else {
          root.classList.add("light");
        }
      } else {
        root.classList.add(savedTheme);
      }
    } else {
      // Default to system preference
      if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
        root.classList.add("dark");
      } else {
        root.classList.add("light");
      }
    }
  } catch {
    // Default to light if loading fails
    root.classList.add("light");
  }

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}

initApp();
