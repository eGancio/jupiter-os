import type { ServiceInfo } from "../../types";
import { getMoonColor } from "../../lib/moonInfo";

interface Props {
  services: ServiceInfo[];
  selected: string | null;
  onSelect: (name: string) => void;
  onInfoClick: (name: string) => void;
}

export function MoonList({ services, selected, onSelect, onInfoClick }: Props) {
  const servers = services.filter((s) => s.kind === "McpServer");
  const daemons = services.filter((s) => s.kind === "Daemon");

  return (
    <>
      {servers.length > 0 && (
        <div className="mb-3">
          <h3 className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim px-3 mb-1">
            Moons
          </h3>
          <div className="px-2">
            {servers.map((svc) => (
              <button
                key={svc.name}
                onClick={() => onSelect(svc.name)}
                className={`w-full text-left px-3 py-1.5 rounded text-sm flex items-center gap-2 transition-colors ${
                  selected === svc.name
                    ? "bg-jupiter-elevated text-white"
                    : "text-white hover:bg-jupiter-elevated"
                }`}
              >
                <span
                  className={`w-2 h-2 rounded-full flex-shrink-0 ${
                    svc.running ? "bg-jupiter-green" : "bg-jupiter-red"
                  }`}
                />
                {svc.name}
                <span
                  onClick={(e) => {
                    e.stopPropagation();
                    onInfoClick(svc.name);
                  }}
                  className="ml-auto text-xs cursor-pointer transition-opacity opacity-70 hover:opacity-100"
                  style={{ color: getMoonColor(svc.name) }}
                  title="Info"
                >
                  &#9432;
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
      {daemons.length > 0 && (
        <div className="mb-3">
          <h3 className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim px-3 mb-1">
            Daemons
          </h3>
          <div className="px-2">
            {daemons.map((svc) => (
              <button
                key={svc.name}
                onClick={() => onSelect(svc.name)}
                className={`w-full text-left px-3 py-1.5 rounded text-sm flex items-center gap-2 transition-colors ${
                  selected === svc.name
                    ? "bg-jupiter-elevated text-white"
                    : "text-white hover:bg-jupiter-elevated"
                }`}
              >
                <span
                  className={`w-2 h-2 rounded-full flex-shrink-0 ${
                    svc.running ? "bg-jupiter-green" : "bg-jupiter-red"
                  }`}
                />
                {svc.name}
                <span
                  onClick={(e) => {
                    e.stopPropagation();
                    onInfoClick(svc.name);
                  }}
                  className="ml-auto text-xs cursor-pointer transition-opacity opacity-70 hover:opacity-100"
                  style={{ color: getMoonColor(svc.name) }}
                  title="Info"
                >
                  &#9432;
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </>
  );
}
