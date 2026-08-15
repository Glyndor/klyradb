package manager

import (
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

func TestCreate_invalidPostgresVersion(t *testing.T) {
	m := newTestManager(t)
	cases := []string{
		"",                      // empty
		"../etc/passwd",         // path traversal
		"16; rm -rf /",          // command metacharacters
		"16 --allow-downgrades", // flag injection
		"16\n",                  // control character
		"16beta1",               // non-numeric suffix
		"v16",                   // leading letter
		"16.3.1",                // too many segments for package name
	}
	for _, v := range cases {
		if _, err := m.Create("pg1", "postgres", v, 0); err == nil {
			t.Errorf("expected error for Postgres version %q, got nil", v)
		}
	}
}

func TestCreate_validPostgresVersion(t *testing.T) {
	m := newTestManager(t)
	for _, v := range []string{"14", "15", "16", "16.3", "17", "18", "18.10"} {
		if _, err := m.Create("pg-"+v, "postgres", v, 0); err != nil {
			t.Errorf("expected Postgres version %q to be accepted, got error: %v", v, err)
		}
	}
}

func TestCreate_otherEnginesUnaffectedByVersionRegex(t *testing.T) {
	// The regex only constrains versions used to build OS package names,
	// which today is PostgreSQL alone. For MySQL/MariaDB/Redis/MongoDB the
	// version is just a UI selector; verify it is not rejected here.
	m := newTestManager(t)
	cases := map[string]string{
		"mysql":   "8.4",
		"mariadb": "11.4",
		"redis":   "8.6",
		"mongodb": "8.2.6",
	}
	for dbType, v := range cases {
		if _, err := m.Create("inst-"+dbType, dbType, v, 0); err != nil {
			t.Errorf("expected %s version %q to be accepted, got error: %v", dbType, v, err)
		}
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
