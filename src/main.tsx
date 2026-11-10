import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource-variable/jetbrains-mono";
import "lxgw-wenkai-screen-webfont/lxgwwenkaiscreen.css";
import "./styles/globals.css";
import App from "./App";
import { QuickReply, type QuickReplyPayload } from "./QuickReply";
import { RootErrorBoundary } from "./components/RootErrorBoundary";
import { installOverflowTitles } from "./lib/overflowTitles";

const notice = (window as Window & { __memdockQuickReply?: QuickReplyPayload }).__memdockQuickReply;
if (!notice) installOverflowTitles();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <RootErrorBoundary>
      {notice ? <QuickReply notice={notice} /> : <App />}
    </RootErrorBoundary>
  </React.StrictMode>,
);
