import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { compactTokens, formatTokens } from "./log-usage";
import { RollingNumber } from "./RollingNumber";

/** Rounding is display-only. Detail panels, data tables and accessible text retain exact integers. */
export function TokenNumber({
  value,
  focusable = false,
  animated = false,
}: {
  value?: number | null;
  focusable?: boolean;
  animated?: boolean;
}) {
  const exact = formatTokens(value);
  const short = exact === "—" ? exact : compactTokens(value!);
  const display = animated ? (
    <RollingNumber value={value} kind="compact" />
  ) : (
    short
  );
  if (exact === short) return <span>{display}</span>;
  const content = (
    <>
      <span aria-hidden="true">{display}</span>
      <span className="sr-only">{exact}</span>
    </>
  );
  if (!focusable)
    return (
      <span className="mg-token-number" title={`${exact} Token`}>
        {content}
      </span>
    );
  return (
    <TooltipProvider delayDuration={200}>
      <Tooltip>
        <TooltipTrigger asChild>
          <span tabIndex={0} className="mg-token-number">
            {content}
          </span>
        </TooltipTrigger>
        <TooltipContent className="mg-token-number-tip">
          {exact} Token
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
