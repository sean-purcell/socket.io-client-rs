var io = require('socket.io')();

var port = Number(process.argv[2]) || 0;

io.on('connect', socket => {
    console.log('connection received with id ' + socket.id);
    socket.emit('test', 'hello', {'key': 'value'});
});

io.attach(port);
console.log(io.httpServer.address().port);
