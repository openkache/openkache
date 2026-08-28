-- kv_point_select.lua
-- sysbench workload: point-select on kvbench.kv by exact 32-byte CHAR key,
-- using a server-side PREPARED statement (SELECT v FROM kv WHERE k = ?).
-- Keys: "kvbench:" + 24-digit zero-padded index, uniform over [0, table_size).

sysbench.cmdline.options = {
   table_size = {"number of rows prefilled in kv", 10000000}
}

local PREFIX = "kvbench:"

function thread_init()
   drv = sysbench.sql.driver()
   con = drv:connect()
   stmt = con:prepare("SELECT v FROM kv WHERE k = ?")
   kparam = stmt:bind_create(sysbench.sql.type.CHAR, 32)
   stmt:bind_param(kparam)
end

function thread_done()
   stmt:close()
   con:disconnect()
end

local function make_key(idx)
   return string.format("%s%024d", PREFIX, idx)
end

function event()
   local idx = sysbench.rand.uniform(0, sysbench.opt.table_size - 1)
   kparam:set(make_key(idx))
   stmt:execute()
end
