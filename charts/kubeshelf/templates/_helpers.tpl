{{- define "kubeshelf.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- define "kubeshelf.fullname" -}}
{{- default .Release.Name .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- define "kubeshelf.selector" -}}
app.kubernetes.io/name: {{ include "kubeshelf.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
{{- define "kubeshelf.labels" -}}
{{ include "kubeshelf.selector" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | quote }}
{{- end -}}
{{- define "kubeshelf.serviceAccount" -}}
{{- default (include "kubeshelf.fullname" .) .Values.serviceAccount.name -}}
{{- end -}}
