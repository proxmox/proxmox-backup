Ext.define('PBS.ServerStatus', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsServerStatus',

    title: gettext('ServerStatus'),

    html: "Add Something usefule here ?",

    initComponent: function() {
        var me = this;

	var node_command = function(cmd) {
	    Proxmox.Utils.API2Request({
		params: { command: cmd },
		url: '/nodes/localhost/status',
		method: 'POST',
		waitMsgTarget: me,
		failure: function(response, opts) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		}
	    });
	};

	var restartBtn = Ext.create('Proxmox.button.Button', {
	    text: gettext('Reboot'),
	    dangerous: true,
	    confirmMsg: gettext("Reboot backup server?"),
	    handler: function() {
		node_command('reboot');
	    },
	    iconCls: 'fa fa-undo'
	});

	var shutdownBtn = Ext.create('Proxmox.button.Button', {
	    text: gettext('Shutdown'),
	    dangerous: true,
	    confirmMsg: gettext("Shutdown backup server?"),
	    handler: function() {
		node_command('shutdown');
	    },
	    iconCls: 'fa fa-power-off'
	});

	me.tbar = [ restartBtn, shutdownBtn ];

	me.callParent();
    }

});
