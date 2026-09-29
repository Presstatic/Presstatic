-- Ogni richiesta chiede un articolo diverso dell'archivio: 1, 2, 3 … 2000, poi ricomincia.
local n = 0
request = function()
  n = n + 1
  return wrk.format("GET", "/articolo-" .. ((n - 1) % 2000 + 1) .. "/")
end
