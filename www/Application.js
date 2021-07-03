Ext.define('PBS.Application', {
    extend: 'Ext.app.Application',

    name: 'PBS',
    appProperty: 'app',

    stores: [
	'NavigationStore',
    ],

    layout: 'fit',

    realignWindows: function() {
	var modalwindows = Ext.ComponentQuery.query('window[modal]');
	Ext.Array.forEach(modalwindows, function(item) {
	    item.center();
	});
    },

    logout: function() {
	var me = this;
	Proxmox.Utils.authClear();
	me.changeView('loginview', true);
    },

    changeView: function(view, skipCheck) {
	var me = this;
	PBS.view = view;
	me.view = view;

	if (me.currentView !== undefined) {
	    me.currentView.destroy();
	}

	me.currentView = Ext.create({
	    xtype: view,
	});
	if (skipCheck !== true) {
	    Proxmox.Utils.checked_command(Ext.emptyFn);
	}
    },

    view: 'loginview',

    launch: function() {
	var me = this;
	Ext.on('resize', me.realignWindows);

	var provider = new Ext.state.LocalStorageProvider({ prefix: 'ext-pbs-' });
	Ext.state.Manager.setProvider(provider);

	let isOpenIDLogin = Proxmox.Utils.getOpenIDRedirectionAuthorization() !== undefined;
	let alreadyLoggedIn = Proxmox.Utils.authOK();

	if (isOpenIDLogin || !alreadyLoggedIn) {
	    me.changeView('loginview', true); // show login window if not loggedin
	} else {
	    me.changeView('mainview', true);
	}
    },
});

Ext.application('PBS.Application');
