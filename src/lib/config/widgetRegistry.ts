// Widget registry — defines all available widgets, their classes, and metadata

export type WidgetClass = 'large' | 'small';

export interface WidgetDef {
  id: string;
  label: string;
  widgetClass: WidgetClass;
}

export const WIDGET_DEFS: WidgetDef[] = [
  { id: 'ahi',          label: 'AHI',            widgetClass: 'large' },
  { id: 'speed',        label: 'Speed',          widgetClass: 'small' },
  { id: 'altitude',     label: 'Altitude',       widgetClass: 'small' },
  { id: 'battery',      label: 'Battery',        widgetClass: 'small' },
  { id: 'gps',          label: 'GPS',            widgetClass: 'small' },
  { id: 'compass',      label: 'Compass',        widgetClass: 'large' },
  { id: 'home',         label: 'Home',           widgetClass: 'small' },
  { id: 'rawTelemetry', label: 'Raw Telemetry',  widgetClass: 'small' },
];

export const WIDGET_MAP = new Map(WIDGET_DEFS.map(w => [w.id, w]));

/** Large widget base size in vmin */
export const LARGE_BASE_VMIN = 22.5;
/** Small widget = 60% of large, always square */
export const SMALL_BASE_VMIN = LARGE_BASE_VMIN * 0.6; // 13.5
/** Minimum scale factor before panel is considered full */
export const MIN_SCALE = 0.5;
