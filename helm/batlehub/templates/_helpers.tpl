{{/*
Expand the name of the chart.
*/}}
{{- define "batlehub.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "batlehub.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart label.
*/}}
{{- define "batlehub.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels.
*/}}
{{- define "batlehub.labels" -}}
helm.sh/chart: {{ include "batlehub.chart" . }}
{{ include "batlehub.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels.
*/}}
{{- define "batlehub.selectorLabels" -}}
app.kubernetes.io/name: {{ include "batlehub.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
ServiceAccount name.
*/}}
{{- define "batlehub.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "batlehub.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Name of the Secret holding the config.
*/}}
{{- define "batlehub.secretName" -}}
{{- if .Values.existingSecret }}
{{- .Values.existingSecret }}
{{- else }}
{{- printf "%s-config" (include "batlehub.fullname" .) }}
{{- end }}
{{- end }}

{{/*
Render the full config.toml from values.
*/}}
{{- define "batlehub.config" -}}
[server]
host = {{ .Values.server.host | quote }}
port = {{ .Values.server.port }}
{{- if .Values.server.staticDir }}
static_dir = {{ .Values.server.staticDir | quote }}
{{- end }}
{{- if .Values.server.corsAllowedOrigins }}
cors_allowed_origins = [{{ range .Values.server.corsAllowedOrigins }}{{ . | quote }}, {{ end }}]
{{- end }}

[database]
type = "postgresql"
url  = {{ .Values.database.url | quote }}

{{- if eq .Values.storage.type "s3" }}

[storage]
type   = "s3"
bucket = {{ .Values.storage.s3.bucket | quote }}
region = {{ .Values.storage.s3.region | quote }}
{{- if .Values.storage.s3.endpoint }}
endpoint = {{ .Values.storage.s3.endpoint | quote }}
{{- end }}
{{- if .Values.storage.s3.accessKeyId }}
access_key_id     = {{ .Values.storage.s3.accessKeyId | quote }}
secret_access_key = {{ .Values.storage.s3.secretAccessKey | quote }}
{{- end }}
{{- else }}

[storage]
type = "filesystem"
path = {{ .Values.storage.path | quote }}
{{- end }}

{{- if .Values.auth.tokens }}

[[auth]]
type = "token"
{{- range .Values.auth.tokens }}

[[auth.tokens]]
value = {{ .value | quote }}
role  = {{ .role | quote }}
{{- if .userId }}
user_id = {{ .userId | quote }}
{{- end }}
{{- end }}
{{- end }}

{{- range .Values.auth.oidc }}

[[auth]]
type         = "oidc"
issuer_url   = {{ .issuerUrl | quote }}
client_id    = {{ .clientId | quote }}
client_secret = {{ .clientSecret | quote }}
redirect_uri = {{ .redirectUri | quote }}
{{- if .scopes }}
scopes = [{{ range .scopes }}{{ . | quote }}, {{ end }}]
{{- end }}
{{- if .userIdClaim }}
user_id_claim = {{ .userIdClaim | quote }}
{{- end }}
{{- if .roleClaim }}
role_claim = {{ .roleClaim | quote }}
{{- end }}
{{- if .roleMappings }}

[auth.role_mappings]
{{- range $group, $role := .roleMappings }}
{{ $group | quote }} = {{ $role | quote }}
{{- end }}
{{- end }}
{{- end }}

{{ .Values.registriesRaw }}

{{- if .Values.otel.enabled }}

[otel]
endpoint = {{ .Values.otel.endpoint | quote }}
{{- end }}

{{ .Values.extraConfig }}
{{- end }}
