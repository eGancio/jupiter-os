import { useT } from "../../i18n";

export function StatusBadge({ running }: { running: boolean }) {
  const { t } = useT();
  return (
    <span
      className={`text-[10px] font-bold px-2 py-0.5 rounded uppercase tracking-wide ${
        running
          ? "bg-jupiter-green/15 text-jupiter-green border border-jupiter-green/30"
          : "bg-jupiter-red/15 text-jupiter-red border border-jupiter-red/30"
      }`}
    >
      {running ? t("status.running") : t("status.stopped")}
    </span>
  );
}
