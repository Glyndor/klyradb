package manager

import (
	"bufio"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"regexp"
	"runtime"
	"strconv"
	"sync"
	"time"

	"klyradb/internal/engine"
	"klyradb/internal/mariadb"
	"klyradb/internal/mongodb"
	"klyradb/internal/mysql"
	"klyradb/internal/pg"
	"klyradb/internal/redis"
	"klyradb/internal/store"
)

// pgPackageVersionRE constrains the PostgreSQL version segment that gets
// concatenated into OS package names ("postgresql-<v>" / "postgresql@<v>").
// Versions come from the user via App.CreateInstance and end up as an
// argument to pkexec; the regex keeps them on a safe character set so the
// construction cannot smuggle shell metacharacters, additional flags, or a
// different package prefix into the apt-get / brew invocation.
var pgPackageVersionRE = regexp.MustCompile(`^[0-9]+(\.[0-9]+)?$`)

// defaultPorts are starting port search points per DB type.
var defaultPorts = map[engine.DBType]int{
	engine.TypePostgres: 5432,
	engine.TypeMySQL:    3306,
	engine.TypeMariaDB:  3316, // offset to avoid collision if MySQL also installed
	engine.TypeRedis:    6379,
	engine.TypeMongoDB:  27017,
}

// confExtensions maps DB types to config file extensions.
var confExtensions = map[engine.DBType]string{
	engine.TypeMySQL:   ".cnf",
	engine.TypeMariaDB: ".cnf",
	engine.TypeRedis:   ".conf",
	engine.TypeMongoDB: ".yml",
}

type Manager struct {
	mu        sync.RWMutex
	instances map[string]*engine.Instance
	engines   map[engine.DBType]engine.Engine
	store     *store.Store
	baseDir   string
}

func New(s *store.Store) *Manager {
	baseDir := engine.BaseDir()
	for _, sub := range []string{"data", "logs", "pids", "conf", "engines"} {
		_ = os.MkdirAll(filepath.Join(baseDir, sub), 0o750)
	}
	return &Manager{
		instances: map[string]*engine.Instance{},
		engines: map[engine.DBType]engine.Engine{
			engine.TypePostgres: pg.New(),
			engine.TypeMySQL:    mysql.New(),
			engine.TypeMariaDB:  mariadb.New(),
			engine.TypeRedis:    redis.New(),
			engine.TypeMongoDB:  mongodb.New(),
		},
		store:   s,
		baseDir: baseDir,
	}
}

func (m *Manager) LoadAll() {
	m.mu.Lock()
	defer m.mu.Unlock()
	raw, err := m.store.ReadRaw()
	if err != nil {
		return
	}
	var list []engine.Instance
	if err := json.Unmarshal(raw, &list); err != nil {
		return
	}
	for _, r := range list {
		inst := r
		eng, ok := m.engines[inst.Type]
		if !ok {
			inst.Status = engine.StatusStopped
		} else {
			inst.Status = eng.CheckStatus(&inst)
		}
		m.instances[inst.ID] = &inst
	}
}

func (m *Manager) ListVersions() []engine.Version {
	var out []engine.Version
	for _, eng := range m.engines {
		out = append(out, eng.Versions()...)
	}
	return out
}

func (m *Manager) Instances() []engine.Instance {
	m.mu.RLock()
	defer m.mu.RUnlock()
	out := make([]engine.Instance, 0, len(m.instances))
	for _, i := range m.instances {
		inst := *i
		inst.UpgradeVersion = m.latestAvailable(inst.Type, inst.Version)
		inst.PatchUpdate = m.latestPatch(inst.Type, inst.Version)
		out = append(out, inst)
	}
	return out
}

// latestPatch returns the API's latest patch version for the same major if it
// differs from the currently installed binary version, else "".
func (m *Manager) latestPatch(t engine.DBType, major string) string {
	eng, ok := m.engines[t]
	if !ok {
		return ""
	}
	for _, v := range eng.Versions() {
		if v.Major == major {
			if v.InstalledVersion != "" && v.LatestPatch != "" && v.InstalledVersion != v.LatestPatch {
				return v.LatestPatch
			}
			return ""
		}
	}
	return ""
}

// latestAvailable returns the highest installed version newer than currentVersion
// for the given DB type. Only works for integer-major versions (PostgreSQL).
func (m *Manager) latestAvailable(t engine.DBType, currentVersion string) string {
	eng, ok := m.engines[t]
	if !ok {
		return ""
	}
	cur, err := strconv.Atoi(currentVersion)
	if err != nil {
		return ""
	}
	best := 0
	bestStr := ""
	for _, v := range eng.Versions() {
		if !v.Installed {
			continue
		}
		maj, err := strconv.Atoi(v.Major)
		if err != nil {
			continue
		}
		if maj > cur && maj > best {
			best = maj
			bestStr = v.Major
		}
	}
	return bestStr
}

func (m *Manager) Create(name, dbType, version string, port int) (engine.Instance, error) {
	t := engine.DBType(dbType)
	eng, ok := m.engines[t]
	if !ok {
		return engine.Instance{}, fmt.Errorf("unknown database type: %s", dbType)
	}

	// The version is interpolated into the OS package name for PostgreSQL
	// (postgresql-<v> on apt, postgresql@<v> on brew). Reject anything that
	// doesn't look like a numeric major[.minor] so an unvalidated frontend
	// input cannot reach pkexec as part of a package argument.
	if t == engine.TypePostgres && !pgPackageVersionRE.MatchString(version) {
		return engine.Instance{}, fmt.Errorf("invalid PostgreSQL version %q: must match ^[0-9]+(\\.[0-9]+)?$", version)
	}

	if port == 0 {
		port = m.NextFreePort(t)
	}
	if !engine.PortFree(port) {
		return engine.Instance{}, fmt.Errorf("port %d is already in use by another process", port)
	}
	if m.portTaken(port) {
		return engine.Instance{}, fmt.Errorf("port %d is already used by another KlyraDB instance", port)
	}

	id := randID()
	usr := currentUser()
	dataDir := filepath.Join(m.baseDir, "data", id)
	logFile := filepath.Join(m.baseDir, "logs", id+".log")
	pidFile := filepath.Join(m.baseDir, "pids", id+".pid")

	var confFile string
	if ext, ok := confExtensions[t]; ok {
		confFile = filepath.Join(m.baseDir, "conf", id+ext)
	}

	inst := engine.Instance{
		ID:        id,
		Name:      name,
		Type:      t,
		Version:   version,
		Port:      port,
		DataDir:   dataDir,
		LogFile:   logFile,
		PIDFile:   pidFile,
		ConfFile:  confFile,
		User:      usr,
		Status:    engine.StatusInit,
		CreatedAt: time.Now(),
	}

	// Check if binary is installed; if not, save instance for deferred install.
	binInstalled := false
	for _, v := range eng.Versions() {
		if v.Major == version && v.Installed {
			binInstalled = true
			break
		}
	}
	if !binInstalled {
		inst.Status = engine.StatusNeedsInstall
		m.mu.Lock()
		m.instances[id] = &inst
		m.mu.Unlock()
		m.persist()
		return inst, nil
	}

	if err := eng.Create(&inst); err != nil {
		inst.Status = engine.StatusError
		inst.LastError = err.Error()
		return inst, err
	}

	inst.Status = engine.StatusStopped
	m.mu.Lock()
	m.instances[id] = &inst
	m.mu.Unlock()
	m.persist()
	return inst, nil
}

// Install downloads/installs the DB binary for inst, streams output via progress,
// then initializes the data directory.
func (m *Manager) Install(id string, progress func(string)) error {
	m.mu.RLock()
	inst, ok := m.instances[id]
	m.mu.RUnlock()
	if !ok {
		return fmt.Errorf("instance %s not found", id)
	}

	if engine.SnapDir() != "" {
		return fmt.Errorf("%s is not bundled in the snap package", inst.Type)
	}

	cmd := installCmd(inst.Type, inst.Version)
	if len(cmd) == 0 {
		return fmt.Errorf("automatic install not supported for %s on this OS", inst.Type)
	}

	m.setStatus(id, engine.StatusInstalling, "")
	progress("Installing " + string(inst.Type) + " " + inst.Version + "…")

	// cmd[0] is a constant per OS ("pkexec" or "brew") and the rest of the
	// argument slice is built by installCmd -> linuxPackage/brewPackage, where
	// the only interpolated user input (PostgreSQL version) is matched by
	// pgPackageVersionRE. Defensive validation in linuxPackage/brewPackage
	// returns "" on a malformed version, which installCmd already handles by
	// returning nil (caller errors out before reaching this exec).
	c := exec.Command(cmd[0], cmd[1:]...) //nolint:gosec // see pgPackageVersionRE
	stdout, _ := c.StdoutPipe()
	stderr, _ := c.StderrPipe()

	if err := c.Start(); err != nil {
		m.setStatus(id, engine.StatusNeedsInstall, err.Error())
		return fmt.Errorf("install failed to start: %w", err)
	}

	done := make(chan struct{})
	go func() {
		defer close(done)
		sc := bufio.NewScanner(stdout)
		for sc.Scan() {
			progress(sc.Text())
		}
	}()
	scErr := bufio.NewScanner(stderr)
	for scErr.Scan() {
		progress("! " + scErr.Text())
	}
	<-done

	if err := c.Wait(); err != nil {
		m.setStatus(id, engine.StatusNeedsInstall, err.Error())
		return fmt.Errorf("install failed: %w", err)
	}

	progress("Binary installed. Initializing database…")

	eng := m.engines[inst.Type]
	if err := eng.Create(inst); err != nil {
		m.setStatus(id, engine.StatusError, err.Error())
		return err
	}

	m.setStatus(id, engine.StatusStopped, "")
	progress("Done.")
	return nil
}

// UpgradePatch stops all running instances of the same type+major, runs the
// OS package upgrade, then restarts the instances that were running before.
func (m *Manager) UpgradePatch(id string, progress func(string)) error {
	m.mu.RLock()
	inst, ok := m.instances[id]
	m.mu.RUnlock()
	if !ok {
		return fmt.Errorf("instance %s not found", id)
	}

	if engine.SnapDir() != "" {
		return fmt.Errorf("%s patch upgrade is not supported in the snap package", inst.Type)
	}

	cmd := upgradeCmd(inst.Type, inst.Version)
	if len(cmd) == 0 {
		return fmt.Errorf("automatic upgrade not supported for %s on this OS", inst.Type)
	}

	// Collect running siblings (same type + major).
	m.mu.RLock()
	var wasRunning []string
	for iid, i := range m.instances {
		if i.Type == inst.Type && i.Version == inst.Version && i.Status == engine.StatusRunning {
			wasRunning = append(wasRunning, iid)
		}
	}
	m.mu.RUnlock()

	for _, iid := range wasRunning {
		m.mu.RLock()
		name := m.instances[iid].Name
		m.mu.RUnlock()
		progress("Stopping " + name + "…")
		if err := m.Stop(iid); err != nil {
			progress("! " + err.Error())
		}
	}

	progress("Upgrading " + string(inst.Type) + " " + inst.Version + "…")

	// See Install: cmd[0] is constant per OS, the only user-derived segment
	// (PostgreSQL version) is bounded by pgPackageVersionRE in linuxPackage
	// and brewPackage. A version that fails the regex makes upgradeCmd return
	// nil, and the caller errors out before this exec.
	c := exec.Command(cmd[0], cmd[1:]...) //nolint:gosec // see pgPackageVersionRE
	stdout, _ := c.StdoutPipe()
	stderr, _ := c.StderrPipe()

	if err := c.Start(); err != nil {
		for _, iid := range wasRunning {
			_ = m.Start(iid)
		}
		return fmt.Errorf("upgrade failed to start: %w", err)
	}

	done := make(chan struct{})
	go func() {
		defer close(done)
		sc := bufio.NewScanner(stdout)
		for sc.Scan() {
			progress(sc.Text())
		}
	}()
	scErr := bufio.NewScanner(stderr)
	for scErr.Scan() {
		progress("! " + scErr.Text())
	}
	<-done

	if err := c.Wait(); err != nil {
		for _, iid := range wasRunning {
			_ = m.Start(iid)
		}
		return fmt.Errorf("upgrade failed: %w", err)
	}

	progress("Binary upgraded. Restarting instances…")
	for _, iid := range wasRunning {
		m.mu.RLock()
		name := m.instances[iid].Name
		m.mu.RUnlock()
		progress("Starting " + name + "…")
		if err := m.Start(iid); err != nil {
			progress("! Failed to start " + name + ": " + err.Error())
		}
	}
	progress("Done.")
	return nil
}

func upgradeCmd(t engine.DBType, version string) []string {
	switch runtime.GOOS {
	case "linux":
		pkg := linuxPackage(t, version)
		if pkg == "" {
			return nil
		}
		return []string{"pkexec", "apt-get", "install", "-y", "--only-upgrade", pkg}
	case "darwin":
		pkg := brewPackage(t, version)
		if pkg == "" {
			return nil
		}
		return []string{"brew", "upgrade", pkg}
	}
	return nil
}

func (m *Manager) setStatus(id string, s engine.Status, lastErr string) {
	m.mu.Lock()
	if inst, ok := m.instances[id]; ok {
		inst.Status = s
		inst.LastError = lastErr
	}
	m.mu.Unlock()
	m.persist()
}

// installCmd returns the OS-appropriate command to install a DB binary.
func installCmd(t engine.DBType, version string) []string {
	switch runtime.GOOS {
	case "linux":
		pkg := linuxPackage(t, version)
		if pkg == "" {
			return nil
		}
		return []string{"pkexec", "apt-get", "install", "-y", pkg}
	case "darwin":
		pkg := brewPackage(t, version)
		if pkg == "" {
			return nil
		}
		return []string{"brew", "install", pkg}
	}
	return nil
}

func linuxPackage(t engine.DBType, version string) string {
	switch t {
	case engine.TypePostgres:
		if !pgPackageVersionRE.MatchString(version) {
			return ""
		}
		return "postgresql-" + version
	case engine.TypeMySQL:
		return "mysql-server"
	case engine.TypeMariaDB:
		return "mariadb-server"
	case engine.TypeRedis:
		return "redis-server"
	case engine.TypeMongoDB:
		return "mongodb"
	}
	return ""
}

func brewPackage(t engine.DBType, version string) string {
	switch t {
	case engine.TypePostgres:
		if !pgPackageVersionRE.MatchString(version) {
			return ""
		}
		return "postgresql@" + version
	case engine.TypeMySQL:
		return "mysql"
	case engine.TypeMariaDB:
		return "mariadb"
	case engine.TypeRedis:
		return "redis"
	case engine.TypeMongoDB:
		return "mongodb-community"
	}
	return ""
}

func (m *Manager) Start(id string) error {
	m.mu.Lock()
	inst, ok := m.instances[id]
	m.mu.Unlock()
	if !ok {
		return fmt.Errorf("instance %s not found", id)
	}
	eng, ok := m.engines[inst.Type]
	if !ok {
		return fmt.Errorf("no engine for type %s", inst.Type)
	}
	if err := eng.Start(inst); err != nil {
		m.mu.Lock()
		inst.Status = engine.StatusError
		inst.LastError = err.Error()
		m.mu.Unlock()
		m.persist()
		return err
	}
	m.mu.Lock()
	inst.Status = engine.StatusRunning
	inst.LastError = ""
	m.mu.Unlock()
	m.persist()
	return nil
}

func (m *Manager) Stop(id string) error {
	m.mu.RLock()
	inst, ok := m.instances[id]
	m.mu.RUnlock()
	if !ok {
		return fmt.Errorf("instance %s not found", id)
	}
	eng, ok := m.engines[inst.Type]
	if !ok {
		return fmt.Errorf("no engine for type %s", inst.Type)
	}
	if err := eng.Stop(inst); err != nil {
		return err
	}
	m.mu.Lock()
	inst.Status = engine.StatusStopped
	m.mu.Unlock()
	m.persist()
	return nil
}

func (m *Manager) StopAll() {
	m.mu.RLock()
	ids := make([]string, 0)
	for id, i := range m.instances {
		if i.Status == engine.StatusRunning {
			ids = append(ids, id)
		}
	}
	m.mu.RUnlock()
	for _, id := range ids {
		_ = m.Stop(id)
	}
}

func (m *Manager) Delete(id string) error {
	m.mu.Lock()
	inst, ok := m.instances[id]
	if !ok {
		m.mu.Unlock()
		return fmt.Errorf("instance %s not found", id)
	}
	delete(m.instances, id)
	m.mu.Unlock()

	if eng, ok := m.engines[inst.Type]; ok {
		_ = eng.Delete(inst)
	}
	m.persist()
	return nil
}

func (m *Manager) Status(id string) (engine.Status, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	inst, ok := m.instances[id]
	if !ok {
		return "", fmt.Errorf("instance %s not found", id)
	}
	if eng, ok := m.engines[inst.Type]; ok {
		inst.Status = eng.CheckStatus(inst)
	}
	return inst.Status, nil
}

// NextFreePort finds the first free port starting from the type's default,
// skipping ports already used by other instances.
func (m *Manager) NextFreePort(t engine.DBType) int {
	start, ok := defaultPorts[t]
	if !ok {
		start = 15432
	}
	for p := start; p < start+500; p++ {
		if engine.PortFree(p) && !m.portTaken(p) {
			return p
		}
	}
	return start
}

// ---- internals ----

func (m *Manager) portTaken(p int) bool {
	for _, i := range m.instances {
		if i.Port == p {
			return true
		}
	}
	return false
}

func (m *Manager) persist() {
	m.mu.RLock()
	list := make([]engine.Instance, 0, len(m.instances))
	for _, i := range m.instances {
		list = append(list, *i)
	}
	m.mu.RUnlock()
	b, err := json.MarshalIndent(list, "", "  ")
	if err != nil {
		return
	}
	_ = m.store.WriteRaw(b)
}

func randID() string {
	b := make([]byte, 6)
	_, _ = rand.Read(b)
	return hex.EncodeToString(b)
}

func currentUser() string {
	if u := os.Getenv("USER"); u != "" {
		return u
	}
	if u := os.Getenv("LOGNAME"); u != "" {
		return u
	}
	if u, err := user.Current(); err == nil {
		return u.Username
	}
	return "klyradb"
}
