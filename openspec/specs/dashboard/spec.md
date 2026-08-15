# Dashboard Specification

## Purpose

A React web dashboard for searching crawl results and monitoring crawler health, served as a static build behind nginx.

## Requirements

### Requirement: Search page
The dashboard SHALL provide an instant search interface: a search input whose query is debounced, with filter controls (size range, age) and sort selection, rendering results with pagination.

#### Scenario: Typing triggers search
- **WHEN** the user types in the search box
- **THEN** a search request fires after the debounce, and results render without a full page reload

#### Scenario: Filters and sort reflected
- **WHEN** the user changes a filter or sort control
- **THEN** results refresh with the new parameters

### Requirement: Monitoring page
The dashboard SHALL render crawler monitoring from the admin API: a live summary header (verified, unique, fetch activity, memory), time-series charts for key metrics (verified/hr, unique/hr, routing nodes, memory), and system resource charts (network bandwidth, cpu, memory, disk), plus a failure breakdown view.

#### Scenario: Live summary renders
- **WHEN** the monitoring page loads
- **THEN** the latest snapshot summary is displayed with the headline metrics

#### Scenario: Time-series charts render
- **WHEN** history is loaded for a metric
- **THEN** a chart renders the metric over the selected range

#### Scenario: Failure breakdown renders
- **WHEN** the monitoring page requests failures
- **THEN** failure counts by reason are displayed

### Requirement: State management
Dashboard UI state (search query/filters/sort/pagination, monitoring selections) SHALL be managed in client stores so navigation between pages preserves the user's selections.

#### Scenario: Store persists selections
- **WHEN** the user moves from search to monitoring and back
- **THEN** their search query and filter selections are preserved

### Requirement: Strict TypeScript
Dashboard source SHALL compile under strict TypeScript with no implicit any, and the production build SHALL pass type-checking and linting before serving.

#### Scenario: Type-safe API client
- **WHEN** the dashboard calls either API
- **THEN** responses are typed and validated against the API contract
