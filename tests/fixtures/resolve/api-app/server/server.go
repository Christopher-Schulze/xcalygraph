package server

type userServiceServer struct{}

// FetchUser implements the rpc.
func (s *userServiceServer) FetchUser(id string) string { return id }

// GetUser serves GET /users/{id}.
func GetUser(id string) string { return id }
