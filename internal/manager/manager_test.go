package manager

import (
	"strings"
	"testing"

	"klyradb/internal/engine"
	"klyradb/internal/store"
)

// mockEngine satisfies engine.Engine without touching real DB binaries.
type mockEngine struct {
	dbType engine.DBType
}

func (m *mockEngine) DBType() engine.DBType { return m.dbType }
func (m *mockEngine) Versions() []engine.Version {
	return []engine.Version{
		{Type: m.dbType, Major: "14", Installed: true},
		{Type: m.dbType, Major: "15", Installed: true},
		{Type: m.dbType, Major: "16", Installed: true},
	}
}
func (m *mockEngine) Create(inst *engine.Instance) error { return nil }
func (m *mockEngine) Start(inst *engine.Instance) error  { return nil }
func (m *mockEngine) Stop(inst *engine.Instance) error   { return nil }
func (m *mockEngine) Delete(inst *engine.Instance) error { return nil }
func (m *mockEngine) CheckStatus(inst *engine.Instance) engine.Status {
	return engine.StatusStopped
}

func newTestManager(t *testing.T) *Manager {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("SNAP_USER_COMMON", dir)
	s, err := store.New()
	if err != nil {
		t.Fatalf("store.New: %v", err)
	}
	return &Manager{
		instances: map[string]*engine.Instance{},
		engines: map[engine.DBType]engine.Engine{
			engine.TypePostgres: &mockEngine{dbType: engine.TypePostgres},
			engine.TypeMySQL:    &mockEngine{dbType: engine.TypeMySQL},
			engine.TypeMariaDB:  &mockEngine{dbType: engine.TypeMariaDB},
			engine.TypeRedis:    &mockEngine{dbType: engine.TypeRedis},
			engine.TypeMongoDB:  &mockEngine{dbType: engine.TypeMongoDB},
		},
		store:   s,
		baseDir: dir,
	}
}

func TestNextFreePort_isFree(t *testing.T) {
	m := newTestManager(t)
	for _, dbType := range []engine.DBType{engine.TypePostgres, engine.TypeMySQL, engine.TypeMariaDB, engine.TypeRedis, engine.TypeMongoDB} {
		p := m.NextFreePort(dbType)
		if !engine.PortFree(p) {
			t.Errorf("%s: NextFreePort returned %d which is not free", dbType, p)
		}
	}
}

func TestNextFreePort_skipsTakenInstance(t *testing.T) {
	m := newTestManager(t)
	m.instances["existing"] = &engine.Instance{ID: "existing", Type: engine.TypePostgres, Port: 5432}
	p := m.NextFreePort(engine.TypePostgres)
	if p == 5432 {
		t.Error("NextFreePort should skip port already used by another instance")
	}
}

func TestPortTaken(t *testing.T) {
	m := newTestManager(t)
	m.instances["a"] = &engine.Instance{ID: "a", Port: 9876}
	if !m.portTaken(9876) {
		t.Error("port 9876 should be taken")
	}
	if m.portTaken(9877) {
		t.Error("port 9877 should not be taken")
	}
}

func TestLatestAvailable_returnsNewest(t *testing.T) {
	m := newTestManager(t)
	got := m.latestAvailable(engine.TypePostgres, "14")
	if got != "16" {
		t.Errorf("expected 16, got %s", got)
	}
}

func TestLatestAvailable_noneNewer(t *testing.T) {
	m := newTestManager(t)
	got := m.latestAvailable(engine.TypePostgres, "16")
	if got != "" {
		t.Errorf("expected empty (already at latest), got %s", got)
	}
}

func TestLatestAvailable_nonNumericVersion(t *testing.T) {
	m := newTestManager(t)
	got := m.latestAvailable(engine.TypePostgres, "notanumber")
	if got != "" {
		t.Errorf("expected empty for non-numeric current version, got %s", got)
	}
}

func TestCreate_unknownType(t *testing.T) {
	m := newTestManager(t)
	if _, err := m.Create("test", "unknown_db", "1", 0); err == nil {
		t.Error("expected error for unknown DB type")
	}
}

func TestLinuxPackage_mongodbIsBlocked(t *testing.T) {
	if got := linuxPackage(engine.TypeMongoDB, "8.2.6"); got != "" {
		t.Errorf("linuxPackage(MongoDB) should return \"\" so installCmd short-circuits, got %q", got)
	}
}

func TestLinuxPackage_otherEnginesUnchanged(t *testing.T) {
	cases := map[engine.DBType]string{
		engine.TypeMySQL:   "mysql-server",
		engine.TypeMariaDB: "mariadb-server",
		engine.TypeRedis:   "redis-server",
	}
	for dbType, want := range cases {
		if got := linuxPackage(dbType, ""); got != want {
			t.Errorf("linuxPackage(%s) = %q, want %q", dbType, got, want)
		}
	}
}

func TestLinuxInstallBlockedReason_mongodbExplainsTheLimitation(t *testing.T) {
	reason := linuxInstallBlockedReason(engine.TypeMongoDB, "8.2.6")
	if reason == "" {
		t.Fatal("expected a non-empty reason for MongoDB on Linux")
	}
	// The message must answer the three questions: what, why, what-to-do.
	wantSubstrings := []string{
		"MongoDB on Linux",
		"repo.mongodb.org", // where the official package lives
		"klyradb does not", // why klyradb can't do it for you
		"mongodb-org",      // the right package name
		"rewrite",          // the upcoming verified-download path
	}
	for _, sub := range wantSubstrings {
		if !strings.Contains(reason, sub) {
			t.Errorf("MongoDB reason missing %q; full message:\n%s", sub, reason)
		}
	}
}

func TestLinuxInstallBlockedReason_otherEnginesReturnEmpty(t *testing.T) {
	for _, dbType := range []engine.DBType{
		engine.TypePostgres, engine.TypeMySQL, engine.TypeMariaDB, engine.TypeRedis,
	} {
		if got := linuxInstallBlockedReason(dbType, ""); got != "" {
			t.Errorf("linuxInstallBlockedReason(%s) = %q, want \"\"", dbType, got)
		}
	}
}

func TestInstall_mongodbOnLinuxReturnsExplanatoryError(t *testing.T) {
	m := newTestManager(t)
	inst, err := m.Create("pg-first", "postgres", "16", 0)
	if err != nil {
		t.Fatalf("Create postgres: %v", err)
	}
	mongo, err := m.Create("mongo-1", "mongodb", "8.2.6", 0)
	if err != nil {
		t.Fatalf("Create mongodb: %v", err)
	}
	// Run the install on the MongoDB instance. We are not running under Snap
	// in tests; this exercises the direct-download path. The package
	// manager must not be invoked — the function must fail closed with the
	// specific reason before reaching installCmd.
	err = m.Install(mongo.ID, func(string) {})
	if err == nil {
		t.Fatal("expected Install(MongoDB,Linux) to fail, got nil")
	}
	if !strings.Contains(err.Error(), "MongoDB on Linux") {
		t.Errorf("expected the specific MongoDB-on-Linux reason, got: %v", err)
	}
	if strings.Contains(err.Error(), "Unable to locate package") {
		t.Errorf("install must not reach apt — got a raw apt message: %v", err)
	}
	// The lastError on the persisted instance must carry the user-facing
	// reason so the frontend card (line ~264 of main.js) can show it.
	// (Manager.Status() overwrites Status with eng.CheckStatus(), so we
	// assert on LastError directly.)
	m.mu.RLock()
	last := m.instances[mongo.ID].LastError
	m.mu.RUnlock()
	if !strings.Contains(last, "MongoDB on Linux") {
		t.Errorf("instance.LastError should carry the specific reason, got %q", last)
	}
	// Sanity: the Postgres instance is untouched.
	if _, ok := m.instances[inst.ID]; !ok {
		t.Error("the unrelated Postgres instance should not be affected by the MongoDB failure")
	}
}

func TestUpgradePatch_mongodbOnLinuxReturnsExplanatoryError(t *testing.T) {
	m := newTestManager(t)
	mongo, err := m.Create("mongo-1", "mongodb", "8.2.6", 0)
	if err != nil {
		t.Fatalf("Create mongodb: %v", err)
	}
	err = m.UpgradePatch(mongo.ID, func(string) {})
	if err == nil {
		t.Fatal("expected UpgradePatch(MongoDB,Linux) to fail, got nil")
	}
	if !strings.Contains(err.Error(), "MongoDB on Linux") {
		t.Errorf("expected the specific MongoDB-on-Linux reason, got: %v", err)
	}
	if strings.Contains(err.Error(), "Unable to locate package") {
		t.Errorf("upgrade must not reach apt — got a raw apt message: %v", err)
	}
}

func TestStart_notFound(t *testing.T) {
	m := newTestManager(t)
	if err := m.Start("no-such-id"); err == nil {
		t.Error("expected error for nonexistent instance")
	}
}

func TestStop_notFound(t *testing.T) {
	m := newTestManager(t)
	if err := m.Stop("no-such-id"); err == nil {
		t.Error("expected error for nonexistent instance")
	}
}

func TestDelete_notFound(t *testing.T) {
	m := newTestManager(t)
	if err := m.Delete("no-such-id"); err == nil {
		t.Error("expected error for nonexistent instance")
	}
}

func TestStatus_notFound(t *testing.T) {
	m := newTestManager(t)
	if _, err := m.Status("no-such-id"); err == nil {
		t.Error("expected error for nonexistent instance")
	}
}

func TestCreate_andDelete(t *testing.T) {
	m := newTestManager(t)
	inst, err := m.Create("mydb", "postgres", "16", 0)
	if err != nil {
		t.Fatalf("Create: %v", err)
	}
	if inst.ID == "" {
		t.Error("expected non-empty ID")
	}
	if inst.Port == 0 {
		t.Error("expected non-zero port")
	}
	if inst.Name != "mydb" {
		t.Errorf("expected name mydb, got %s", inst.Name)
	}
	if err := m.Delete(inst.ID); err != nil {
		t.Fatalf("Delete: %v", err)
	}
	if _, err := m.Status(inst.ID); err == nil {
		t.Error("expected error querying deleted instance")
	}
}

func TestCreate_portConflict(t *testing.T) {
	m := newTestManager(t)
	inst, err := m.Create("first", "postgres", "16", 0)
	if err != nil {
		t.Fatalf("first Create: %v", err)
	}
	second, err := m.Create("second", "postgres", "16", 0)
	if err != nil {
		t.Fatalf("second Create: %v", err)
	}
	if inst.Port == second.Port {
		t.Errorf("two instances got same port %d", inst.Port)
	}
}

func TestLoadAll_roundtrip(t *testing.T) {
	m := newTestManager(t)
	if _, err := m.Create("pg1", "postgres", "15", 0); err != nil {
		t.Fatalf("Create: %v", err)
	}

	m2 := &Manager{
		instances: map[string]*engine.Instance{},
		engines:   m.engines,
		store:     m.store,
		baseDir:   m.baseDir,
	}
	m2.LoadAll()

	list := m2.Instances()
	if len(list) != 1 {
		t.Fatalf("expected 1 instance after reload, got %d", len(list))
	}
	if list[0].Name != "pg1" {
		t.Errorf("expected name pg1, got %s", list[0].Name)
	}
}

func TestListVersions_allEngines(t *testing.T) {
	m := newTestManager(t)
	versions := m.ListVersions()
	if len(versions) == 0 {
		t.Error("expected versions from all engines")
	}
	// 5 engines × 3 versions each = 15
	if len(versions) != 15 {
		t.Errorf("expected 15 versions (5 engines × 3), got %d", len(versions))
	}
}

func TestInstances_upgradeVersionPopulated(t *testing.T) {
	m := newTestManager(t)
	if _, err := m.Create("pg1", "postgres", "14", 0); err != nil {
		t.Fatalf("Create: %v", err)
	}
	list := m.Instances()
	if len(list) != 1 {
		t.Fatalf("expected 1 instance, got %d", len(list))
	}
	if list[0].UpgradeVersion != "16" {
		t.Errorf("expected upgrade to 16, got %q", list[0].UpgradeVersion)
	}
}

func TestStopAll_onlyStopsRunning(t *testing.T) {
	m := newTestManager(t)
	inst, err := m.Create("pg1", "postgres", "15", 0)
	if err != nil {
		t.Fatalf("Create: %v", err)
	}
	// mark as running manually
	m.mu.Lock()
	m.instances[inst.ID].Status = engine.StatusRunning
	m.mu.Unlock()

	m.StopAll()

	m.mu.RLock()
	status := m.instances[inst.ID].Status
	m.mu.RUnlock()
	if status != engine.StatusStopped {
		t.Errorf("expected stopped after StopAll, got %s", status)
	}
}
