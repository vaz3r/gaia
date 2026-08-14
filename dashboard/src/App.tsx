import { NavLink, Route, Routes } from "react-router-dom";

import { MonitoringPage } from "@/features/monitoring/MonitoringPage";
import { SearchPage } from "@/features/search/SearchPage";
import { cn } from "@/lib/utils";

const navItems = [
  { to: "/", label: "Search", end: true },
  { to: "/monitoring", label: "Monitoring", end: false },
];

export default function App(): JSX.Element {
  return (
    <div className="min-h-screen">
      <header className="border-b">
        <div className="mx-auto flex max-w-7xl items-center gap-6 px-6 py-3">
          <span className="text-lg font-bold">gaia</span>
          <nav className="flex gap-2">
            {navItems.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                end={item.end}
                className={({ isActive }) =>
                  cn(
                    "rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                    isActive
                      ? "bg-primary text-primary-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  )
                }
              >
                {item.label}
              </NavLink>
            ))}
          </nav>
        </div>
      </header>
      <main className="mx-auto max-w-7xl px-6 py-6">
        <Routes>
          <Route path="/" element={<SearchPage />} />
          <Route path="/monitoring" element={<MonitoringPage />} />
        </Routes>
      </main>
    </div>
  );
}
