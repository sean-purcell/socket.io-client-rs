var io = require('socket.io')();

var port = Number(process.argv[2]) || 0;

io.on('connect', socket => {
    console.error('connection received with id ' + socket.id);
    socket.emit('test', 'hello', {'key': 'value'});
    socket.binary(true).emit('binary', Buffer.from([0xde, 0xad, 0xbe, 0xef]), Buffer.from([0xde, 0xad, 0xbe, 0xef]));
    socket.binary(false).emit('binary', Buffer.from([0xde, 0xad, 0xbe, 0xef]));
    socket.emit('types', [0,1,2], {'key': 'value'}, 'hello', 4, (ack) => console.log);
});

io.of('/nsp').on('connect', socket => {
    console.error('connection to /nsp received');
    socket.binary(true).emit('binary namespaced message with ack', (ack) => console.log);
});

io.attach(port);
console.log(io.httpServer.address().port);
