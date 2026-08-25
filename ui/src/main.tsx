// The real frontend entry point (r1.s1.w5), replacing the placeholder
// `r1.s1.w1` built to be thrown away. Mounts the ported design system's
// `App` (chrome + sidebar + the no-credential banner) into `#root`.
//
// `tokens.css` is imported before `shared.css`, matching the load order
// `docs/design/mock/README.md` documents ("Every screen links tokens.css
// then shared.css first"); `app-shell.css` is this app's own small addition
// (not from the mock -- see that file's header).

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./styles/tokens.css";
import "./styles/shared.css";
import "./styles/app-shell.css";

function element(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (found === null) {
    throw new Error(`index.html is missing #${id}`);
  }
  return found;
}

createRoot(element("root")).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
