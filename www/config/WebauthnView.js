Ext.define('PBS.WebauthnConfigView', {
    extend: 'Proxmox.grid.ObjectGrid',
    alias: ['widget.pbsWebauthnConfigView'],

    url: "/api2/json/config/access/tfa/webauthn",
    cwidth1: 150,
    interval: 1000,

    rows: {
	rp: {
	    header: gettext('Relying Party'),
	    required: true,
	},
	origin: {
	    header: gettext('Origin'),
	    required: true,
	},
	id: {
	    header: gettext('Id'),
	    required: true,
	},
    },

    tbar: [
	{
	    text: gettext("Edit"),
	    handler: 'runEditor',
	},
    ],
    controller: {
	xclass: 'Ext.app.ViewController',

	runEditor: function() {
	    let win = Ext.create('PBS.WebauthnConfigEdit');
	    win.show();
	},

	startStore: function() { this.getView().getStore().rstore.startUpdate(); },
	stopStore: function() { this.getView().getStore().rstore.stopUpdate(); },
    },


    listeners: {
	itemdblclick: 'runEditor',
	activate: 'startStore',
	deactivate: 'stopStore',
	destroy: 'stopStore',
    },
});

Ext.define('PBS.WebauthnConfigEdit', {
    extend: 'Proxmox.window.Edit',
    alias: ['widget.pbsWebauthnConfigEdit'],

    subject: gettext('Webauthn'),
    url: "/api2/extjs/config/access/tfa/webauthn",
    autoLoad: true,

    fieldDefaults: {
	labelWidth: 120,
    },

    items: [
	{
	    xtype: 'textfield',
	    fieldLabel: gettext('Relying Party'),
	    name: 'rp',
	    allowBlank: false,
	},
	{
	    xtype: 'textfield',
	    fieldLabel: gettext('Origin'),
	    name: 'origin',
	    allowBlank: false,
	},
	{
	    xtype: 'textfield',
	    fieldLabel: gettext('id'),
	    name: 'id',
	    allowBlank: false,
	},
    ],
});
