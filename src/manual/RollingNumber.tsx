import { memo, useSyncExternalStore } from "react";
import NumberFlow, { useIsSupported, type Format } from "@number-flow/react";
const reducedQuery = () =>
  typeof window === "undefined"
    ? undefined
    : window.matchMedia?.("(prefers-reduced-motion: reduce)");
const subscribeMotion = (notify: () => void) => {
  const query = reducedQuery();
  query?.addEventListener?.("change", notify);
  return () => query?.removeEventListener?.("change", notify);
};

export type NumberKind = "count" | "compact" | "percent" | "ratio";
const formats: Record<NumberKind, Format> = {
  count: { maximumFractionDigits: 0 },
  compact: { notation: "compact", maximumFractionDigits: 2 },
  percent: {
    style: "percent",
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  },
  ratio: {
    style: "percent",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  },
};
const timing = { duration: 380, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" };
const opacityTiming = { duration: 160, easing: "ease-out" };
export const RollingNumber = memo(function RollingNumber({
  value,
  kind = "count",
  className = "",
}: {
  value?: number | null;
  kind?: NumberKind;
  className?: string;
}) {
  const supported = useIsSupported();
  const reduced = useSyncExternalStore(
    subscribeMotion,
    () => reducedQuery()?.matches ?? false,
    () => false,
  );
  const canAnimate = supported && !reduced;
  const known =
    typeof value === "number" && Number.isFinite(value) && value >= 0;
  const text = known
    ? new Intl.NumberFormat("en-US", formats[kind]).format(value)
    : "—";
  return (
    <span
      className={`mg-rolling ${className}`}
      data-number-text={text}
      data-number-kind={kind}
    >
      {!known ? (
        "—"
      ) : canAnimate ? (
        <NumberFlow
          value={value}
          locales="en-US"
          format={formats[kind]}
          transformTiming={timing}
          spinTiming={timing}
          opacityTiming={opacityTiming}
          respectMotionPreference
          animated={typeof document === "undefined" || !document.hidden}
        />
      ) : (
        new Intl.NumberFormat("en-US", formats[kind])
          .formatToParts(value)
          .map((part, index) =>
            part.type === "compact" || part.type === "percentSign" ? (
              <span className={`mg-static-unit is-${part.type}`} key={index}>
                {part.value}
              </span>
            ) : (
              part.value
            ),
          )
      )}
    </span>
  );
});
