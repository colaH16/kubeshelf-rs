// Command reference reads one JSON test case from stdin and prints the
// original Go implementation's answer as JSON. It is built by
// scripts/build_reference.py from the pinned original commit only.
package main

import (
	"encoding/json"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/colaH16/KubeShelf/internal/shelf"
)

type input struct {
	Op       string
	Snapshot shelf.Snapshot
	Settings shelf.Settings
	Old      shelf.Settings
	Identity shelf.Identity
	Profile  shelf.FavoriteProfile
	Before   shelf.FavoriteProfile
	Route    string
	Raw      string
	GitURL   string
	Branch   string
	GitPath  string
	Env      map[string]string
	Schemes  map[string]string
}

func errorText(err error) any {
	if err == nil {
		return nil
	}
	return err.Error()
}

func main() {
	c := shelf.Config{Demo: true, Cluster: "cluster", IngressSchemes: map[string]string{"demo-https": "https"}}
	if len(os.Args) > 1 && os.Args[1] == "demo" {
		snap := shelf.NewDemoSource().Snapshot()
		snap.UpdatedAt = time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
		_ = json.NewEncoder(os.Stdout).Encode(map[string]any{"snapshot": snap, "settings": shelf.ReferenceDemoSettings(c)})
		return
	}
	var in input
	if err := json.NewDecoder(os.Stdin).Decode(&in); err != nil {
		panic(err)
	}
	if in.Schemes != nil {
		c.IngressSchemes = in.Schemes
	}
	var out any
	switch in.Op {
	case "catalog":
		out = shelf.BuildCatalog(in.Snapshot, in.Settings, c, in.Identity)
	case "settings":
		if err := shelf.ValidateSettings(&in.Settings); err != nil {
			out = map[string]any{"error": err.Error()}
		} else {
			out = map[string]any{"settings": in.Settings, "version": shelf.ReferenceVersion(in.Settings)}
		}
	case "decode-settings":
		s, err := shelf.ReferenceDecodeSettings([]byte(in.Raw))
		if err != nil {
			out = map[string]any{"error": err.Error()}
		} else {
			out = map[string]any{"settings": s, "version": shelf.ReferenceVersion(s)}
		}
	case "favorites":
		if err := shelf.ReferenceValidateFavorites(&in.Profile); err != nil {
			out = map[string]any{"error": err.Error()}
		} else {
			out = shelf.ReferenceFavorites(in.Profile, in.Identity)
		}
	case "default-favorites":
		out = shelf.ReferenceFavorites(shelf.ReferenceDefaultFavorites(in.Identity.Admin), in.Identity)
	case "route":
		out = shelf.ReferenceSafeReturnTo(in.Route)
	case "url":
		result := map[string]any{"valid": shelf.ReferenceValidateURL(in.Raw) == nil}
		if u, err := url.Parse(in.Raw); err == nil {
			result["string"] = u.String()
			result["scheme"] = u.Scheme
			result["hostname"] = u.Hostname()
			// The catalog re-renders discovered and manual URLs with a new scheme.
			u.Scheme = "https"
			result["https"] = u.String()
		}
		out = result
	case "host":
		out = map[string]any{"error": errorText(shelf.ReferenceValidateHost(in.Raw))}
	case "change-summary":
		out = shelf.ReferenceChangeSummary(in.Old, in.Settings)
	case "personal-summary":
		out = shelf.ReferencePersonalSummary(in.Before, in.Profile)
	case "legacy-defaults":
		out = shelf.ReferenceSameLegacyDefaults(in.Old, in.Settings)
	case "encode-settings":
		b, err := shelf.ReferenceEncodeSettings(in.Settings)
		out = map[string]any{"yaml": string(b), "error": errorText(err)}
	case "encode-favorites":
		b, err := shelf.ReferenceEncodeFavorites(in.Identity, in.Profile)
		out = map[string]any{"yaml": string(b), "error": errorText(err)}
	case "favorite-path":
		out = shelf.ReferenceFavoritePath(in.GitPath, in.Identity)
	case "read-settings":
		s, err := shelf.ReferenceReadSettingsManifest(in.GitPath, []byte(in.Raw))
		if err != nil {
			out = map[string]any{"error": err.Error()}
		} else {
			out = map[string]any{"settings": s, "version": shelf.ReferenceVersion(s)}
		}
	case "read-favorites":
		p, exists, err := shelf.ReferenceReadFavoritesManifest(in.GitPath, in.Identity, []byte(in.Raw))
		if err != nil {
			out = map[string]any{"error": err.Error()}
		} else {
			out = map[string]any{"profile": p, "exists": exists}
		}
	case "store-validation":
		out = shelf.ReferenceOpenStoreError(in.GitURL, in.Branch, in.GitPath)
	case "config":
		for _, kv := range os.Environ() {
			if k, _, _ := strings.Cut(kv, "="); strings.HasPrefix(k, "KUBESHELF_") {
				_ = os.Unsetenv(k)
			}
		}
		for k, v := range in.Env {
			_ = os.Setenv(k, v)
		}
		cfg, err := shelf.ConfigFromEnv()
		if err != nil {
			out = map[string]any{"error": err.Error()}
			break
		}
		if cfg.Demo { // cmd/kubeshelf/main.go adds this scheme in demo mode.
			cfg.IngressSchemes["demo-https"] = "https"
		}
		out = cfg
	default:
		panic("unknown op " + in.Op)
	}
	_ = json.NewEncoder(os.Stdout).Encode(out)
}
