import React from "react";
import ReactDOM from "react-dom/client";
import { createHashRouter, RouterProvider } from "react-router";
import App from "./App";
import PspLibrary from "./pages/PspLibrary";
import Settings from "./pages/Settings";
import ThirdPartyApps from "./pages/ThirdPartyApps";
import { InstancesProvider } from "./lib/instances-context";
import { ModeProvider } from "./lib/mode-context";
import "./index.css";

const router = createHashRouter([
  {
    path: "/",
    Component: App,
    children: [
      { index: true, Component: PspLibrary },
      { path: "third-party", Component: ThirdPartyApps },
      { path: "settings", Component: Settings },
    ],
  },
]);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ModeProvider>
      <InstancesProvider>
        <RouterProvider router={router} />
      </InstancesProvider>
    </ModeProvider>
  </React.StrictMode>,
);
