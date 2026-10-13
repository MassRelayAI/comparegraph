package store

type Repo struct{}

func NewRepo() *Repo { return &Repo{} }

func (r *Repo) Put() { r.save() }

func (r *Repo) save() {}
