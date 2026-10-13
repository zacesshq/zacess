import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { loadCore, errorMessage } from "@/core";
import App from "./App";
import "./index.css";

const container = document.getElementById("root")!;

loadCore()
  .then(() => {
    createRoot(container).render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
  })
  .catch((e: unknown) => {
    // the app cannot run without its core, so say so plainly rather than show a blank page
    container.innerHTML = "";
    const main = document.createElement("main");
    const heading = document.createElement("h1");
    heading.textContent = "Zacess could not start";
    const text = document.createElement("p");
    text.textContent = `This browser could not load the part of Zacess that reads documents (${errorMessage(e)}). A recent version of Chrome, Edge, Firefox or Safari is needed.`;
    main.append(heading, text);
    container.append(main);
  });
