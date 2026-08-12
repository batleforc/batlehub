/** A single trail entry. `to` is omitted for the current page. */
export interface Crumb {
  label: string;
  to?: string;
}
