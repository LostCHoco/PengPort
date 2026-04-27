import React from "react";
import ReactDOM from "react-dom/client";
import { createHashRouter, RouterProvider } from "react-router";
import App from "./App";
import PspLibrary from "./pages/PspLibrary";
import Settings from "./pages/Settings";
import "./index.css";

const router = createHashRouter([
  {
    path: "/",
    Component: App,
    children: [
      { index: true, Component: PspLibrary },
      { path: "settings", Component: Settings },
    ],
  },
]);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>,
);
