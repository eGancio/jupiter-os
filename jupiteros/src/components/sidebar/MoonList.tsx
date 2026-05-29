import type { ServiceInfo } from "../../types";
import { getMoonColor, getMoonInfo } from "../../lib/moonInfo";
import { useT } from "../../i18n";

interface Props {
  services: ServiceInfo[];
  selected: string | null;
  onSelect: (name: string) => void;
  onInfoClick: (name: string) => void;
  onAddMoon: () => void;
  onRemoveMoon: (name: string) => void;
}

export function MoonList({ services, selected, onSelect, onInfoClick, onAddMoon, onRemoveMoon }: Props) {
  const { t } = useT();
  const servers = services.filter((s) => s.kind === "McpServer");
  const daemons = services.filter((s) => s.kind === "Daemon");

  return (
    <>
      {(servers.length > 0 || true) && (
        <div className="mb-3">
          <div className="flex items-center px-3 mb-1">
            <h3 className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim flex-1">
              {t("moonlist.moons")}
            </h3>
            <button
              onClick={onAddMoon}
              className="text-jupiter-dim hover:text-jupiter-orange text-[14px] leading-none transition-colors"
              title={t("moonlist.addLocal")}
            >
              +
            </button>
          </div>
          <div className="px-2">
            {servers.map((svc) => {
              const isOfficial = getMoonInfo(svc.name) !== null;
              return (
                <button
                  key={svc.name}
                  onClick={() => onSelect(svc.name)}
                  className={`group w-full text-left px-3 py-1.5 rounded text-sm flex items-center gap-2 transition-colors ${
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
                  <span className="truncate flex-1">{svc.name}</span>
                  {!isOfficial && (
                    <span
                      onClick={(e) => {
                        e.stopPropagation();
                        onRemoveMoon(svc.name);
                      }}
                      className="opacity-0 group-hover:opacity-70 hover:!opacity-100 text-jupiter-red text-xs cursor-pointer transition-opacity"
                      title={t("moonlist.remove")}
                    >
                      ×
                    </span>
                  )}
                  <span
                    onClick={(e) => {
                      e.stopPropagation();
                      onInfoClick(svc.name);
                    }}
                    className="text-xs cursor-pointer transition-opacity opacity-70 hover:opacity-100"
                    style={{ color: getMoonColor(svc.name) }}
                    title={t("moonlist.info")}
                  >
                    &#9432;
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
      {daemons.length > 0 && (
        <div className="mb-3">
          <h3 className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim px-3 mb-1">
            {t("moonlist.daemons")}
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
                  title={t("moonlist.info")}
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
