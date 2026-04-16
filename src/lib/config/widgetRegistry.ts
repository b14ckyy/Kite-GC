// Widget registry — defines all available widgets, their classes, and metadata

export type WidgetClass = 'large' | 'small';

export interface WidgetDef {
  id: string;
  label: string;
  /** i18n key for the label — use with $t() in .svelte files */
  labelKey: string;
  widgetClass: WidgetClass;
}

export const WIDGET_DEFS: WidgetDef[] = [
  { id: 'ahi',          label: 'AHI',            labelKey: 'widgets.ahi',          widgetClass: 'large' },
  { id: 'speed',        label: 'Speed',          labelKey: 'widgets.speed',        widgetClass: 'small' },
  { id: 'altitude',     label: 'Altitude',       labelKey: 'widgets.altitude',     widgetClass: 'small' },
  { id: 'battery',      label: 'Battery',        labelKey: 'widgets.battery',      widgetClass: 'small' },
  { id: 'gps',          label: 'GPS',            labelKey: 'widgets.gps',          widgetClass: 'small' },
  { id: 'compass',      label: 'Compass',        labelKey: 'widgets.compass',      widgetClass: 'large' },
  { id: 'home',         label: 'Home',           labelKey: 'widgets.home',         widgetClass: 'small' },
  { id: 'rawTelemetry', label: 'Raw Telemetry',  labelKey: 'widgets.rawTelemetry', widgetClass: 'small' },
];

export const WIDGET_MAP = new Map(WIDGET_DEFS.map(w => [w.id, w]));

/** Large widget base size in vmin */
export const LARGE_BASE_VMIN = 22.5;
/** Small widget = 60% of large, always square */
export const SMALL_BASE_VMIN = LARGE_BASE_VMIN * 0.6; // 13.5
/** Minimum scale factor before panel is considered full */
export const MIN_SCALE = 0.5;
