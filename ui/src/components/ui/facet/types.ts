/** One filter choice. `count` is optional — not every facet has totals. */
export interface FacetOption {
  value: string;
  label: string;
  count?: number;
}
