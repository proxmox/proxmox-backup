Ext.define('pbs-authids', {
    extend: 'Ext.data.Model',
    fields: [
	'authid', 'comment', 'type',
    ],
    idProperty: 'authid',
});

Ext.define('PBS.form.AuthidSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsAuthidSelector',

    allowBlank: false,
    autoSelect: false,
    valueField: 'authid',
    displayField: 'authid',

    editable: true,
    anyMatch: true,
    forceSelection: true,

    store: {
	model: 'pbs-authids',
	params: {
	    enabled: 1,
	},
	sorters: 'authid',
    },

    initComponent: function() {
	let me = this;
	me.userStore = Ext.create('Ext.data.Store', {
	    model: 'pbs-users-with-tokens',
	});
	me.userStore.on('load', this.onLoad, this);
	me.userStore.load();

	me.callParent();
    },

    onLoad: function(store, data, success) {
	if (!success) return;

	let authidStore = this.store;

	let records = [];
	Ext.Array.each(data, function(user) {
	let u = {};
	u.authid = user.data.userid;
	u.comment = user.data.comment;
	u.type = 'u';
	records.push(u);
	let tokens = user.data.tokens || [];
	Ext.Array.each(tokens, function(token) {
	    let r = {};
	    r.authid = token.tokenid;
	    r.comment = token.comment;
	    r.type = 't';
	    records.push(r);
	});
	});

	authidStore.loadData(records);
    },

    listConfig: {
	columns: [
	    {
		header: gettext('Type'),
		sortable: true,
		dataIndex: 'type',
		renderer: function(value) {
		    switch (value) {
			case 'u': return gettext('User');
			case 't': return gettext('API Token');
			default: return Proxmox.Utils.unknownText;
		    }
		},
		flex: 1,
	    },
	    {
		header: gettext('Auth ID'),
		sortable: true,
		dataIndex: 'authid',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		sortable: false,
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },
});

