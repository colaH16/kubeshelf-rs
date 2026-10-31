// Differential-test adapter for the original Go implementation.
//
// scripts/build_reference.py copies this file into a temporary export of the
// pinned original commit as internal/shelf/export_reference.go. It only wraps
// unexported original functions; it never changes their behaviour, and it is
// never added to the original repository.
package shelf

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func ReferenceDemoSettings(c Config) Settings                        { return demoSettings(c) }
func ReferenceVersion(s Settings) string                             { return settingsVersion(s) }
func ReferenceFavorites(p FavoriteProfile, u Identity) FavoriteState { return favoriteState(p, u) }
func ReferenceDefaultFavorites(admin bool) FavoriteProfile           { return defaultFavorites(admin) }
func ReferenceValidateFavorites(p *FavoriteProfile) error            { return validateFavoriteProfile(p) }
func ReferenceSafeReturnTo(s string) string                          { return safeReturnTo(s) }
func ReferenceValidateURL(s string) error                            { return validateURL(s) }
func ReferenceValidateHost(s string) error                           { return validateHost(s) }
func ReferenceDecodeSettings(b []byte) (Settings, error)             { return decodeSettings(b) }
func ReferenceChangeSummary(a, b Settings) string                    { return changeSummary(a, b) }
func ReferencePersonalSummary(a, b FavoriteProfile) string           { return personalChangeSummary(a, b) }
func ReferenceSameLegacyDefaults(a, b Settings) bool                 { return sameLegacyDefaults(a, b) }
func ReferenceEncodeSettings(s Settings) ([]byte, error)             { return encodeSettings(s) }
func ReferenceEncodeFavorites(u Identity, p FavoriteProfile) ([]byte, error) {
	return encodeFavorites(u, p)
}
func ReferenceFavoritePath(gitPath string, u Identity) string {
	return (&gitRepository{cfg: Config{GitPath: gitPath}}).favoritePath(u)
}

// ReferenceOpenStoreError runs the original OpenStore validation sequence with
// an unreadable SSH key, so the first Git command fails before any network use.
// Callers compare only whether the error is one of the validation messages.
func ReferenceOpenStoreError(gitURL, branch, gitPath string) string {
	dir, err := os.MkdirTemp("", "kubeshelf-ref-store-")
	if err != nil {
		panic(err)
	}
	defer os.RemoveAll(dir)
	settings := filepath.Join(dir, "settings.json")
	if err := os.WriteFile(settings, []byte(`{"schemaVersion":1,"revision":"initial"}`), 0o600); err != nil {
		panic(err)
	}
	cfg := Config{SettingsFile: settings, WorkDir: dir, GitURL: gitURL, GitBranch: branch, GitPath: gitPath, SSHKeyFile: filepath.Join(dir, "missing-key")}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	if _, err := OpenStore(ctx, cfg); err != nil {
		return err.Error()
	}
	return ""
}

// commitManifest creates a throwaway local repository containing one file.
// The original read paths (`git show`, `ls-tree`, strict YAML decoding and
// target checks) then run unchanged against it.
func commitManifest(path string, data []byte) (*gitRepository, string, func()) {
	dir, err := os.MkdirTemp("", "kubeshelf-ref-git-")
	if err != nil {
		panic(err)
	}
	cleanup := func() { os.RemoveAll(dir) }
	git := func(cwd string, args ...string) string {
		cmd := exec.Command("git", append([]string{"-c", "user.name=Reference", "-c", "user.email=reference@example.invalid", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"}, args...)...)
		cmd.Dir = cwd
		cmd.Env = append(os.Environ(), "GIT_CONFIG_NOSYSTEM=1", "GIT_CONFIG_GLOBAL=/dev/null")
		out, err := cmd.Output()
		if err != nil {
			panic(err)
		}
		return string(out)
	}
	bare := filepath.Join(dir, "remote.git")
	seed := filepath.Join(dir, "seed")
	git(dir, "init", "--bare", "--initial-branch=main", bare)
	git(dir, "init", "--initial-branch=main", seed)
	if err := os.MkdirAll(filepath.Dir(filepath.Join(seed, path)), 0o700); err != nil {
		panic(err)
	}
	if err := os.WriteFile(filepath.Join(seed, path), data, 0o600); err != nil {
		panic(err)
	}
	git(seed, "add", "-A")
	git(seed, "commit", "-m", "seed")
	git(seed, "push", bare, "main")
	repo := &gitRepository{cfg: Config{GitURL: bare, GitBranch: "main", GitPath: path, WorkDir: dir}, dir: filepath.Join(dir, "repository"), allowLocal: true}
	ctx := context.Background()
	if err := repo.init(ctx); err != nil {
		panic(err)
	}
	// The same fetch the original read path performs before reading blobs.
	if _, err := repo.run(ctx, nil, "fetch", "--depth=20", "origin", "main"); err != nil {
		panic(err)
	}
	return repo, git(seed, "rev-parse", "HEAD")[:40], cleanup
}

func ReferenceReadSettingsManifest(gitPath string, data []byte) (Settings, error) {
	repo, _, cleanup := commitManifest(gitPath, data)
	defer cleanup()
	s, _, err := repo.read(context.Background())
	return s, err
}

func ReferenceReadFavoritesManifest(gitPath string, u Identity, data []byte) (FavoriteProfile, bool, error) {
	probe := &gitRepository{cfg: Config{GitPath: gitPath}}
	repo, head, cleanup := commitManifest(probe.favoritePath(u), data)
	defer cleanup()
	repo.cfg.GitPath = gitPath
	return repo.readFavorites(context.Background(), u, head)
}
